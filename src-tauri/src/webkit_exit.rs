//! Ends WebKit's web process before it runs its own exit (Linux).
//!
//! WebKitGTK's web process crashes in its exit handlers while freeing its GPU
//! buffer device (`libgbm`, Mesa's `dri_gbm.so`), on every quit since at
//! least 18 November 2026, leaving a crash report each time. 1-1 stops its
//! web processes first, when the window is about to close or the app is
//! asked to exit. Browser storage lives in WebKit's network process, which is
//! left alone, so nothing is lost.

/// Web process children of this process: `(pid, parent pid, name)` from
/// each `/proc/<pid>/stat`.
pub fn web_processes(own: i32, stats: impl Iterator<Item = (i32, String)>) -> Vec<i32> {
    stats
        .filter_map(|(pid, stat)| {
            // "pid (comm) state ppid ...": comm may contain spaces, so split
            // at the last parenthesis.
            let open = stat.find('(')?;
            let close = stat.rfind(')')?;
            let name = &stat[open + 1..close];
            let ppid: i32 = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
            (ppid == own && name.starts_with("WebKitWebProces")).then_some(pid)
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub fn end_web_processes() {
    let own = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let stats = entries.filter_map(|entry| {
        let pid: i32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
        Some((
            pid,
            std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?,
        ))
    });
    for pid in web_processes(own, stats) {
        log::info!("Ending WebKit web process {pid} before it exits on its own");
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_web_processes() {
        let stats = vec![
            (10, "10 (WebKitWebProces) S 1 1 1".to_string()),
            (11, "11 (WebKitWebProces) S 7 7 7".to_string()),
            (12, "12 (WebKitNetworkP) S 1 1 1".to_string()),
            (13, "13 (odd (name)) S 1 1 1".to_string()),
            (14, "garbage".to_string()),
        ];
        assert_eq!(web_processes(1, stats.into_iter()), vec![10]);
    }
}
