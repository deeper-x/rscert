Create a **private key and a self-signed root certificate** for an internal certificate authority (CA).

### Stage 1:
```sh
openssl genrsa -aes256 -out internal-root-ca.key 4096
```

- Generates a **4096-bit RSA private key**.
- Encrypts the key file with **AES-256**, protected by a passphrase OpenSSL prompts you to enter.
- Saves it as `internal-root-ca.key`.

### Stage 2:
```sh
openssl req -x509 -new -sha256 -days 3650 \
  -key internal-root-ca.key \
  -out internal-root-ca.crt \
  -subj "/CN=My Company Internal Root CA"
```

- `req -x509 -new`: creates a new **self-signed certificate**, rather than a certificate signing request.
- `-sha256`: uses SHA-256 when signing the certificate.
- `-days 3650`: makes the certificate valid for 3,650 days, roughly 10 years.
- `-key`: uses the private key you just created; you’ll be prompted for its passphrase.
- `-out`: saves the certificate as `internal-root-ca.crt`.
- `-subj`: sets its identity, with the Common Name (`CN`) “My Company Internal Root CA”.

The key is secret; the certificate is shareable. 
Systems that explicitly trust this root certificate can trust certificates issued under it, subject to normal certificate validation.

Note: these commands do not explicitly set CA extensions. Whether the certificate includes `basicConstraints = CA:TRUE` depends on your OpenSSL configuration. 
A root CA setup should explicitly configure that extension and appropriate key usage, such as `keyCertSign` and `cRLSign`.

### Stage 3:
Generate server.txt:
```sh
subjectAltName = DNS:staging.service.cloud
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
```

Description:
`server.ext` defines certificate extensions for a TLS server certificate. OpenSSL can apply them when signing the server’s certificate request.

- **`subjectAltName = DNS:staging.service.cloud`**
  Identifies the hostname the certificate is valid for. Browsers and TLS clients check this field against the hostname they connect to. It covers exactly `staging.service.cloud`.

- **`keyUsage = critical, digitalSignature, keyEncipherment`**
  Allows the key to create digital signatures and encrypt key material. Signatures are used for authentication in modern TLS; key encipherment supports older RSA key exchange. `critical` means a client must understand and enforce this extension or reject the certificate.

- **`extendedKeyUsage = serverAuth`**
  Declares that the certificate is intended to authenticate a TLS server.

These extensions describe the server certificate’s identity and permitted uses. Clients still need to trust the CA that signs it.


### Stage 4:
Create and sign the staging certificate:
```
openssl genrsa -out staging.key 2048

openssl req -new -key staging.key \
  -out staging.csr \
  -subj "/CN=staging.service.cloud"

openssl x509 -req -sha256 -days 90 \
  -in staging.csr \
  -CA internal-root-ca.crt \
  -CAkey internal-root-ca.key \
  -CAcreateserial \
  -out staging.crt \
  -extfile server.ext
```

### Stage 5:
Generate the staging server’s private key, request a certificate, and sign it using your internal CA.

```sh
openssl genrsa -out staging.key 2048
```

Creates a 2048-bit RSA private key in `staging.key`. Unlike your CA key, this key is not encrypted with a passphrase, so the server can load it automatically. Protect it with restrictive file permissions.

```sh
openssl req -new -key staging.key \
  -out staging.csr \
  -subj "/CN=staging.service.cloud"
```

### Stage 6:
Creates a certificate signing request (CSR) containing the server’s public key and the Common Name `staging.service.cloud`. The request is signed with `staging.key` to prove possession of the corresponding private key; it does not contain that private key.

```sh
openssl x509 -req -sha256 -days 90 \
  -in staging.csr \
  -CA internal-root-ca.crt \
  -CAkey internal-root-ca.key \
  -CAcreateserial \
  -out staging.crt \
  -extfile server.ext
```

Issues the server certificate:

- `-req -in staging.csr`: reads the CSR.
- `-sha256 -days 90`: signs using SHA-256 and sets validity to 90 days.
- `-CA`: uses your root certificate as the issuer.
- `-CAkey`: signs with your CA’s private key, prompting for its passphrase.
- `-CAcreateserial`: creates a CA serial-number file if needed, normally `internal-root-ca.srl`.
- `-out staging.crt`: saves the signed certificate.
- `-extfile server.ext`: adds the hostname and TLS server usage extensions you defined earlier.

Configure the staging server with `staging.crt` and `staging.key`. Clients must trust `internal-root-ca.crt` to accept the certificate. Keep the CA’s private key off the staging server.


### Stage 7:
Then install `internal-root-ca.crt` into each managed client’s trust store:

- Debian/Ubuntu: copy it to `/usr/local/share/ca-certificates/internal-root-ca.crt`, then run `sudo update-ca-certificates`.

Installing `internal-root-ca.crt` tells the client to trust certificates issued by your internal CA.

On Debian/Ubuntu, copy the certificate and rebuild the system’s trusted certificate bundle:

```sh
sudo cp internal-root-ca.crt /usr/local/share/ca-certificates/
sudo update-ca-certificates
```

Applications that use the system trust store can then trust your staging certificate. Some applications maintain separate trust stores and need separate configuration.


### Stage 8:
Validate from a trusted client:

```
openssl s_client \
  -connect staging.service.cloud:443 \
  -servername staging.service.cloud \
  -CAfile internal-root-ca.crt
```

- `-connect`: connects to the staging server on HTTPS port 443.
- `-servername`: sends the hostname using **SNI**, allowing the server to select the correct certificate.
- `-CAfile`: explicitly trusts your root certificate for this test—even if it hasn’t been installed in the system trust store.

Look for:

```text
Verify return code: 0 (ok)
```

This command does not check that the certificate matches the hostname. For hostname validation and to stop on certificate verification errors, use:

```sh
openssl s_client \
  -connect staging.service.cloud:443 \
  -servername staging.service.cloud \
  -CAfile internal-root-ca.crt \
  -verify_hostname staging.service.cloud \
  -verify_return_error
```

To test the installed system trust store instead, omit `-CAfile`; OpenSSL will use its default trust locations.
