# Hosts that accept only legacy RDP security

Some hosts refuse TLS and NLA and accept only Standard RDP Security, the encryption RDP had before 2003. Windows hosts do this when the policy "Require use of specific security layer for remote (RDP) connections" is set to **RDP**. 1-1 then reports: "This host accepts only legacy RDP security".

To connect anyway, open the host's settings and set **Security** to **Legacy**. On the command line the flag is `--security legacy`.

## What you give up

| | Automatic (TLS, NLA) | Legacy |
|---|---|---|
| Encryption | TLS 1.2 or 1.3 | RC4 with a 128-, 56- or 40-bit key |
| Integrity | TLS | an 8-byte MD5/SHA-1 signature |
| The host proves who it is | certificate, confirmed fingerprint | no |
| You prove who you are before a session exists | yes (NLA) | no: the host draws its sign-in screen for anyone who reaches it |
| Password on the wire | never sent (NLA) | sent, RC4-encrypted, when you connect with a saved password |

The missing proof of the host is the serious part. Anyone on the network path between you and the host can pose as the host, read the password and the screen, and change what you type. RC4 itself is also considered broken.

Use Legacy only where the network path is trusted, for example inside a VPN that itself encrypts and authenticates. The better fix is on the host: set the security layer to **Negotiate** or **SSL (TLS 1.0)** and require NLA.

## What works differently

- **Shared folders are not offered.** 1-1 gives a folder only to a host whose identity it checked.
- **The server identity review does not apply.** There is no certificate to confirm.
- **Test credentials** cannot verify a password, because that check uses NLA.
- **FIPS-level hosts** (3DES) are not supported; 1-1 says so when it meets one.
