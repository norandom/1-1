//! Task 3.2: drafts bind host, trusted key and folder identity; confirmation
//! rejects stale, retargeted or replaced selections and saves under the
//! library lock; revoking is idempotent.
use super::*;

#[test]
fn suggested_names_are_portable() {
    for (folder, expected) in [
        (r"D:\work", "work"),
        (r"D:\My Projects", "MyProje"),
        (r"D:\ünï", "n"),
        (r"D:\ÄÖÜ", "share"),
        (r"D:\build-1_x", "build-1"),
    ] {
        let name = suggested_share_name(std::path::Path::new(folder));
        assert_eq!(name, expected, "{folder}");
        assert!(is_portable_share_name(&name));
    }
}

#[test]
fn trust_errors_point_to_the_right_next_step() {
    let changed = trust_message(
        io::Error::new(io::ErrorKind::PermissionDenied, "x"),
        "h:3389",
    );
    assert!(changed.contains("changed since you confirmed it"));
    let unreachable = trust_message(io::Error::new(io::ErrorKind::TimedOut, "x"), "h:3389");
    assert!(unreachable.starts_with("Could not reach h:3389"));
    let untrusted = trust_message(io::Error::other("unknown issuer"), "h:3389");
    assert!(untrusted.contains("Review server identity first"));
}

#[cfg(windows)]
mod windows_build {
    use super::*;
    use crate::rdp::peer_identity::ConnectionTarget;
    use crate::store::connections::ConnectionInput;

    struct Fixture {
        dir: tempfile::TempDir,
        library: LibraryStore,
        grants: Arc<ShareGrantStore>,
        drafts: FolderShareDrafts,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let library = LibraryStore::open(dir.path().join("connections.json"));
            let grants = Arc::new(ShareGrantStore::new(dir.path().join("share-grants.json")));
            library.attach_share_grants(Arc::clone(&grants));
            Self {
                dir,
                library,
                grants,
                drafts: FolderShareDrafts::default(),
            }
        }

        fn host(&self, host: &str) -> ConnectionConfig {
            let input: ConnectionInput = serde_json::from_value(
                serde_json::json!({ "name": "Build", "host": host, "port": 3389 }),
            )
            .unwrap();
            self.library.create_connection(input).unwrap()
        }

        fn folder(&self, name: &str) -> PathBuf {
            let folder = real_path(self.dir.path()).join(name);
            std::fs::create_dir_all(&folder).unwrap();
            folder
        }

        /// The draft `select_share_folder` makes after its picker and handshake.
        fn draft(&self, host: &ConnectionConfig, folder: &std::path::Path) -> Uuid {
            let snapshot = self.library.select_host_for_review(&host.id).unwrap();
            let endpoint = ConnectionTarget::new(&snapshot.host, snapshot.port)
                .unwrap()
                .endpoint;
            self.drafts.insert(Draft {
                host: snapshot,
                peer: ObservedPeer {
                    endpoint,
                    spki_sha256: [5; 32],
                },
                folder: folder.to_path_buf(),
                identity: native::inspect(folder).unwrap(),
                expires: Instant::now() + DRAFT_LIFETIME,
            })
        }

        fn confirm(
            &self,
            selection: Uuid,
            name: &str,
            mode: ShareMode,
        ) -> Result<FolderShareView, String> {
            confirm(
                &self.drafts,
                &self.library,
                &self.grants,
                selection,
                name,
                mode,
            )
        }
    }

    #[test]
    fn a_confirmed_draft_saves_the_host_key_folder_name_and_mode_together() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let folder = fixture.folder("shared");
        let selection = fixture.draft(&host, &folder);
        let view = fixture
            .confirm(selection, "work", ShareMode::ReadWrite)
            .unwrap();
        let shown = view.grant.unwrap();
        assert_eq!(shown.endpoint, "build.example:3389");
        assert_eq!(shown.fingerprint, fingerprint(&[5; 32]));
        assert_eq!(shown.share_name, "work");
        assert_eq!(shown.mode, ShareMode::ReadWrite);
        let saved = fixture.grants.get(&host.id).unwrap().unwrap();
        let identity = native::inspect(&folder).unwrap();
        assert_eq!(
            (saved.volume_id, saved.directory_file_id),
            (identity.volume_id, identity.file_id)
        );
        assert_eq!(saved.root_path, folder);
        assert_eq!(
            fixture.confirm(selection, "work", ShareMode::ReadOnly),
            Err(STALE.into()),
            "a draft confirms once"
        );
    }

    #[test]
    fn an_invalid_name_keeps_the_draft_for_a_corrected_retry() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let selection = fixture.draft(&host, &fixture.folder("shared"));
        assert!(
            fixture
                .confirm(selection, "too long name", ShareMode::ReadOnly)
                .is_err()
        );
        assert!(fixture.grants.get(&host.id).unwrap().is_none());
        assert!(
            fixture
                .confirm(selection, "ok", ShareMode::ReadOnly)
                .is_ok()
        );
    }

    #[test]
    fn an_expired_or_cancelled_draft_cannot_be_confirmed() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let folder = fixture.folder("shared");
        let expired = fixture.draft(&host, &folder);
        fixture
            .drafts
            .0
            .lock()
            .unwrap()
            .get_mut(&expired)
            .unwrap()
            .expires = Instant::now();
        assert_eq!(
            fixture.confirm(expired, "work", ShareMode::ReadOnly),
            Err(STALE.into())
        );
        let cancelled = fixture.draft(&host, &folder);
        fixture.drafts.take(cancelled);
        assert_eq!(
            fixture.confirm(cancelled, "work", ShareMode::ReadOnly),
            Err(STALE.into())
        );
        assert!(fixture.grants.get(&host.id).unwrap().is_none());
    }

    #[test]
    fn a_host_edited_after_selection_rejects_the_confirmation() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let selection = fixture.draft(&host, &fixture.folder("shared"));
        let input: ConnectionInput = serde_json::from_value(
            serde_json::json!({ "name": "Build", "host": "elsewhere.example", "port": 3389 }),
        )
        .unwrap();
        fixture.library.update_connection(&host.id, input).unwrap();
        let error = fixture
            .confirm(selection, "work", ShareMode::ReadOnly)
            .unwrap_err();
        assert!(error.contains("changed"), "{error}");
        assert!(fixture.grants.get(&host.id).unwrap().is_none());
    }

    #[test]
    fn a_replaced_folder_at_the_same_path_rejects_the_confirmation() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let folder = fixture.folder("shared");
        let selection = fixture.draft(&host, &folder);
        std::fs::remove_dir(&folder).unwrap();
        std::fs::create_dir(&folder).unwrap();
        let error = fixture
            .confirm(selection, "work", ShareMode::ReadOnly)
            .unwrap_err();
        assert!(error.contains("folder changed"), "{error}");
        assert!(fixture.grants.get(&host.id).unwrap().is_none());
    }

    #[test]
    fn revoking_is_idempotent_and_reports_the_committed_state() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let selection = fixture.draft(&host, &fixture.folder("shared"));
        fixture
            .confirm(selection, "work", ShareMode::ReadOnly)
            .unwrap();
        for _ in 0..2 {
            fixture.grants.revoke([host.id.as_str()]).unwrap();
            assert_eq!(view(&fixture.grants, &host.id).grant, None);
        }
        assert!(view(&fixture.grants, &host.id).supported);
    }

    #[test]
    fn only_local_drive_letter_subfolders_qualify() {
        let fixture = Fixture::new();
        let folder = fixture.folder("shared");
        let identity = native::inspect(&folder).unwrap();
        assert_eq!(
            native::inspect(&folder).unwrap(),
            identity,
            "the identity is stable"
        );
        assert_ne!(native::inspect(&fixture.folder("other")).unwrap(), identity);
        let file = folder.join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        for (path, expected) in [
            (file.clone(), "not a file"),
            (PathBuf::from(r"C:\"), "not the whole drive"),
            (
                PathBuf::from(r"\\server\share\folder"),
                "local drive letter",
            ),
            (PathBuf::from(r"relative\folder"), "local drive letter"),
            (folder.join("missing"), "Could not open"),
        ] {
            let error = native::inspect(&path).unwrap_err();
            assert!(error.contains(expected), "{}: {error}", path.display());
        }
    }

    /// The long, link-free form of a path. CI temp folders can use 8.3 short
    /// names (`RUNNER~1`), which a folder's final path never does.
    fn real_path(path: &std::path::Path) -> PathBuf {
        let resolved = std::fs::canonicalize(path).unwrap();
        let text = resolved.to_string_lossy();
        PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    }

    fn junction(link: &std::path::Path, target: &std::path::Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success(), "mklink /J failed: {output:?}");
    }

    #[test]
    fn links_anywhere_on_the_way_to_the_folder_are_rejected() {
        let fixture = Fixture::new();
        let real = fixture.folder("real");
        std::fs::create_dir(real.join("sub")).unwrap();
        let link = real_path(fixture.dir.path()).join("via");
        junction(&link, &real);
        let error = native::inspect(&link).unwrap_err();
        assert!(error.contains("links or junctions"), "{error}");
        let error = native::inspect(&link.join("sub")).unwrap_err();
        assert!(error.contains("real location"), "{error}");
        assert!(native::inspect(&real.join("sub")).is_ok());
    }

    #[test]
    fn a_failed_save_leaves_sharing_unchanged_and_needs_a_fresh_choice() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let selection = fixture.draft(&host, &fixture.folder("shared"));
        fixture.grants.fail_next_write();
        let error = fixture
            .confirm(selection, "work", ShareMode::ReadOnly)
            .unwrap_err();
        assert!(error.contains("Could not save"), "{error}");
        assert_eq!(view(&fixture.grants, &host.id).grant, None);
        assert_eq!(
            fixture.confirm(selection, "work", ShareMode::ReadOnly),
            Err(STALE.into())
        );
    }

    #[test]
    fn an_edited_host_gets_a_message_about_the_folder_choice() {
        let fixture = Fixture::new();
        let host = fixture.host("build.example");
        let selection = fixture.draft(&host, &fixture.folder("shared"));
        let input: ConnectionInput = serde_json::from_value(
            serde_json::json!({ "name": "Renamed", "host": "build.example", "port": 3389 }),
        )
        .unwrap();
        fixture.library.update_connection(&host.id, input).unwrap();
        let error = fixture
            .confirm(selection, "work", ShareMode::ReadOnly)
            .unwrap_err();
        assert!(error.contains("Choose the folder again"), "{error}");
    }
}
