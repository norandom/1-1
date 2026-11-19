# Trusting a host: server identity

1-1 trusts a host by its server key. Folder sharing and the credential test only work with a host whose key you have confirmed, or whose certificate comes from a certificate authority your PC already trusts (as on a domain-joined server).

## Confirm a host's key

1. In the sidebar, right-click the host (or select it and press Shift+F10) and choose **Review server identity**.
2. 1-1 connects without credentials and shows the host's address and the **observed fingerprint**, for example `SHA256:3q2+7w…`.
3. Get the fingerprint on the server itself (below) and compare. If they match, choose **Confirm fingerprint**.

From then on 1-1 trusts that key at that address. If the server's key changes later, the review dialog says **The server key has changed** and shows both fingerprints, and a shared folder is no longer offered to that host until you review it again.

On a lab machine you control, confirming the observed fingerprint directly is reasonable. The comparison matters most on a network where someone could intercept the connection.

## Get the fingerprint on the server

The fingerprint is `SHA256:` followed by the unpadded base64 of the SHA-256 hash of the certificate's public key (its DER-encoded SubjectPublicKeyInfo). It is not the certificate thumbprint that Windows shows.

### Windows server

In PowerShell 7 or later, as administrator, on the server:

```powershell
$thumb = (Get-CimInstance -Namespace root\cimv2\TerminalServices -ClassName Win32_TSGeneralSetting -Filter "TerminalName='RDP-Tcp'").SSLCertificateSHA1Hash
$cert  = Get-Item "Cert:\LocalMachine\Remote Desktop\$thumb"
'SHA256:' + [Convert]::ToBase64String([Security.Cryptography.SHA256]::HashData($cert.PublicKey.ExportSubjectPublicKeyInfo())).TrimEnd('=')
```

Windows PowerShell 5.1 lacks `ExportSubjectPublicKeyInfo`; use PowerShell 7.

### Linux server (xrdp, or any server with a PEM certificate)

Point the command at the server's certificate file (xrdp's default is shown):

```bash
openssl x509 -in /etc/xrdp/cert.pem -pubkey -noout \
  | openssl pkey -pubin -outform der \
  | openssl dgst -sha256 -binary \
  | base64 | tr -d '='
```

Prefix the output with `SHA256:` to compare it with the dialog.

> These commands have not yet been checked against every server type. If the output does not match the dialog for a server you trust, report it before confirming.
