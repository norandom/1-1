# RDP folder sharing

## Introduction

A developer using the Windows build of 1-1 can grant a remote host access to one local folder. Programs in the remote session can use the resulting drive or mount through ordinary filesystem operations. A newly selected folder defaults to read-only. The maintainer approved the requirements on 18 November 2026.

## Boundary

The first release covers a selected folder and normal file copies. Client settings apply on the next connection. Transport support, permission to expose the folder and proof that the remote OS mounted it are three separate facts. An optimized transfer command and [workspace delta sync](../workspace-delta-sync/requirements.md) have separate delivery decisions.

## Requirements

### 1. Select a folder for one host

1. When a host has no explicit share grant, 1-1 shall open the session with local folder sharing off, including for existing, imported and duplicated host profiles.
2. When the user configures sharing, 1-1 shall identify the host, chosen folder, remote share name and read-only or read/write mode before saving the grant.
3. When the user enables, changes or removes sharing, 1-1 shall show when the change takes effect, including how to end an active grant immediately by disconnecting.
4. If the connected peer cannot be matched to the peer authorized for the share, including first use or a changed destination or identity, 1-1 shall withhold the folder until the user explicitly grants access to that peer.
5. If the share cannot be offered or used, 1-1 shall distinguish a missing or inaccessible local folder from a known server refusal, unsupported redirection or an unknown cause, and keep the desktop usable when the desktop connection succeeds.
6. When the client can confirm only that it offered the folder, 1-1 shall describe it as offered and reserve any claim of successful access for an observed remote file operation.

### 2. Enforce the selected scope

1. While a read-only share is active, 1-1 shall allow listing and reading within the selected folder and reject changes to contents, names, metadata and directory entries.
2. While a read/write share is active, 1-1 shall restrict permitted operations to the selected folder and its contents under the workstation user's filesystem permissions.
3. If a requested name, link, junction or changed path component would resolve outside the granted folder, 1-1 shall deny access to the outside target, including for rename destinations.
4. When a session ends, 1-1 shall release its open share handles and keep its requests and permissions isolated from other sessions and subsequent connections.

### 3. Copy files from ordinary remote applications

1. Where drive redirection is enabled and supported, 1-1 shall expose the selected folder to ordinary remote applications without requiring a 1-1 helper on the host.
2. When a remote program reads a file from the share, 1-1 shall return the requested contents at the requested offsets, including empty files and offsets beyond 4 GiB in a supported large file.
3. When a remote program writes to a read/write share, 1-1 shall support creating directories, copying, renaming and deleting entries and report operation failures to the calling application, including permission denied when local access is refused.
4. When independent requests overlap in time, 1-1 shall associate each completion with its original request and preserve file contents according to the requested operations.
5. If a copy is interrupted by disconnection or local I/O failure, 1-1 shall report failure on outstanding operations without presenting the incomplete transfer as successful. The ordinary copy tool controls its partial-file behavior.

### 4. Transfer measurements

1. When checking compatibility, the test report shall record the Windows client build and each actual server build, redirection policy when known, offered name, observed drive or mount path, read-only write-denial outcome and read/write copy results with byte-for-byte verification in both directions.
2. When exercising the Windows workstation and the FreeRDP-based and IronRDP-based Linux lab targets, the report shall distinguish successful access, explicit policy rejection, unavailable support and inconclusive results. The Linux case shall exercise the mount from a shell using ordinary file tools.
3. When evaluating the ordinary share under load, the report shall record elapsed time, payload size, transferred session bytes and typing response compared with a session with no file transfer for one large file and a tree of small source files at fixed resolution and network conditions. The report shall identify direction, application, cache conditions and observed number of outstanding requests.
4. If segmented copy is proposed after that baseline, the evaluation shall compare one range in flight with bounded parallel ranges on the same file and connection, verify identical destination contents and record throughput, transferred bytes and typing latency. Ship decisions shall use those measurements.
