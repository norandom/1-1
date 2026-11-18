# Share a local folder with an RDP host

## Problem and outcome

A developer wants to copy files between their Windows workstation and a remote coding session using the tools already on the host. The current 1-1 preview supports clipboard text and images. It does not advertise a filesystem device. The first deliverable is one explicitly selected local folder, usable by a Windows file manager or by Linux applications including `ls`, `cp` and build tools.

## Approach and boundary candidates

Use ordinary RDP drive redirection. A host application accesses a filesystem entry and its server driver issues requests to 1-1. The client can service multiple outstanding ranges. The remote application, filesystem and driver together determine the requests. The baseline feature requires no 1-1 component on the host. File clipboard formats and synchronization are separate features. [Microsoft protocol processing](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/1a8715b1-3afc-4bd7-8ec2-9625a9ce9610) · [Range reads](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/3192516d-36a6-47c5-987a-55c214aa0441)

The Windows client owns folder selection, host authorization, access mode and local filesystem confinement. The remote deployment owns its channel policy, mount point and permissions. In the lab, identify the FreeRDP-based and IronRDP-based Linux server builds. `xfreerdp` is a reference client. xrdp documents a configurable FUSE mount. IronRDP's server library exposes optional device-redirection support. Check what each deployed host actually mounts. [xrdp mount configuration](https://github.com/neutrinolabs/xrdp/blob/devel/sesman/sesman.ini.in) · [IronRDP server registration](https://github.com/Devolutions/IronRDP/blob/abfc97b299f04622cb7860f344a35bbea9bd4361/crates/ironrdp-server/src/server.rs#L3676-L3679)

## Existing spec touchpoints and constraints

Reuse the saved host, local settings and connection lifecycle described in the [existing task list](../windows-clearcodec-royalts-e2e/tasks.md). Design review should verify the upstream Windows drive backend against the pinned dependencies, especially directory-root confinement and read-only enforcement. The current connector accepts self-signed certificates without verifying the peer. Resolve how a share grant identifies its intended peer before exposing local files. A successful user login alone does not establish that identity.

## Upstream and downstream work

- Native runtime acceptance uses the Windows client. Use disposable folders on the Windows and two Linux lab hosts for file operations. Record the host implementation and effective mount location.
- Use the ordinary share as the [benchmark for segmented transfer](requirements.md#4-transfer-measurements). A segmented copy tool is a separate candidate if measurements show a latency bottleneck and useful concurrency on the target. It would explicitly request disjoint ranges, bound in-flight data and verify the assembled result. Ordinary mount users keep their existing copy behavior.
- Repeated updates of an existing workspace have a [separate delta-sync proposal](../workspace-delta-sync/requirements.md). Shared access can ship independently of that proposal.
