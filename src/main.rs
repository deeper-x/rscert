#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, ExitCode},
};

const HELP: &str = "rscert --hostname <DNS name> [OPTIONS]

Generate a new internal CA and a TLS server certificate using OpenSSL.

Options:
  --hostname <name>          Server DNS name (required; also used as the CN)
  --out-dir <path>           New output directory (default: certs)
  --ca-name <name>           CA common name (default: My Company Internal Root CA)
  --ca-days <days>           Root validity (default: 3650)
  --server-days <days>       Server validity (default: 90; cannot exceed CA validity)
  --passphrase-file <path>   Read CA passphrase from the first line of a file
  -h, --help                Show this help

Requires openssl on PATH. Without --passphrase-file, OpenSSL prompts for the
CA passphrase during generation and signing. The server key is unencrypted.
The output directory must not already exist. No trust stores are modified.";

struct Options {
    hostname: String,
    out: PathBuf,
    ca_name: String,
    ca_days: u32,
    server_days: u32,
    passphrase: Option<PathBuf>,
}

fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && name.parse::<std::net::IpAddr>().is_err()
}

fn parse(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut args = args.peekable();
    let mut o = Options {
        hostname: String::new(),
        out: "certs".into(),
        ca_name: "My Company Internal Root CA".into(),
        ca_days: 3650,
        server_days: 90,
        passphrase: None,
    };
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(None);
        }
        if !matches!(
            flag.as_str(),
            "--hostname"
                | "--out-dir"
                | "--ca-name"
                | "--ca-days"
                | "--server-days"
                | "--passphrase-file"
        ) {
            return Err(format!("unknown option: {flag}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--hostname" => o.hostname = value,
            "--out-dir" => o.out = value.into(),
            "--ca-name" => o.ca_name = value,
            "--ca-days" => o.ca_days = value.parse().map_err(|_| "invalid CA validity")?,
            "--server-days" => {
                o.server_days = value.parse().map_err(|_| "invalid server validity")?
            }
            "--passphrase-file" => o.passphrase = Some(value.into()),
            _ => unreachable!(),
        }
    }
    if !valid_hostname(&o.hostname) {
        return Err(
            "--hostname must be a valid DNS name (IP addresses and wildcards are not supported)"
                .into(),
        );
    }
    if o.ca_name.is_empty()
        || o.ca_name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(
            "CA name must be nonempty and contain no control characters, slash, or backslash"
                .into(),
        );
    }
    if o.server_days == 0 || o.ca_days == 0 || o.server_days > o.ca_days || o.ca_days > 36500 {
        return Err(
            "validity must be between 1 and 36500 days; server validity cannot exceed CA validity"
                .into(),
        );
    }
    Ok(Some(o))
}

fn openssl(o: &Options, args: &[&str], password_flag: Option<&str>) -> Result<(), String> {
    let mut command = Command::new("openssl");
    command.current_dir(&o.out).arg(args[0]);
    if let (Some(flag), Some(path)) = (password_flag, &o.passphrase) {
        let mut source = std::ffi::OsString::from("file:");
        source.push(path);
        command.arg(flag).arg(source);
    }
    command.args(&args[1..]);
    let status = command
        .status()
        .map_err(|e| format!("could not run openssl: {e}"))?;
    if !status.success() {
        return Err(format!("openssl {} failed ({status})", args[0]));
    }
    Ok(())
}

fn generate(mut o: Options) -> Result<(), String> {
    // Resolve before changing the child process working directory.
    if let Some(path) = &o.passphrase {
        let path =
            fs::canonicalize(path).map_err(|e| format!("cannot access passphrase file: {e}"))?;
        let bytes = fs::read(&path).map_err(|e| format!("cannot read passphrase file: {e}"))?;
        if bytes
            .split(|b| *b == b'\n')
            .next()
            .unwrap_or_default()
            .iter()
            .all(|b| *b == b'\r')
        {
            return Err("passphrase file must have a nonempty first line".into());
        }
        o.passphrase = Some(path);
    }
    let check = Command::new("openssl")
        .arg("version")
        .output()
        .map_err(|e| format!("OpenSSL is required on PATH: {e}"))?;
    if !check.status.success() {
        return Err("openssl version failed".into());
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(&o.out).map_err(|e| {
        format!(
            "cannot create new output directory {}: {e}",
            o.out.display()
        )
    })?;
    let result = (|| {
        fs::write(o.out.join("server.ext"), format!("basicConstraints = critical, CA:FALSE\nsubjectAltName = DNS:{}\nkeyUsage = critical, digitalSignature, keyEncipherment\nextendedKeyUsage = serverAuth\nsubjectKeyIdentifier = hash\nauthorityKeyIdentifier = keyid,issuer\n", o.hostname)).map_err(|e| e.to_string())?;
        openssl(
            &o,
            &["genrsa", "-aes256", "-out", "internal-root-ca.key", "4096"],
            Some("-passout"),
        )?;
        openssl(
            &o,
            &[
                "req",
                "-x509",
                "-new",
                "-sha256",
                "-days",
                &o.ca_days.to_string(),
                "-key",
                "internal-root-ca.key",
                "-out",
                "internal-root-ca.crt",
                "-subj",
                &format!("/CN={}", o.ca_name),
                "-addext",
                "basicConstraints=critical,CA:TRUE,pathlen:0",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
                "-addext",
                "subjectKeyIdentifier=hash",
            ],
            Some("-passin"),
        )?;
        openssl(&o, &["genrsa", "-out", "staging.key", "2048"], None)?;
        openssl(
            &o,
            &[
                "req",
                "-new",
                "-key",
                "staging.key",
                "-out",
                "staging.csr",
                "-subj",
                &format!("/CN={}", o.hostname),
            ],
            None,
        )?;
        openssl(
            &o,
            &[
                "x509",
                "-req",
                "-sha256",
                "-days",
                &o.server_days.to_string(),
                "-in",
                "staging.csr",
                "-CA",
                "internal-root-ca.crt",
                "-CAkey",
                "internal-root-ca.key",
                "-CAcreateserial",
                "-out",
                "staging.crt",
                "-extfile",
                "server.ext",
            ],
            Some("-passin"),
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for key in ["internal-root-ca.key", "staging.key"] {
                fs::set_permissions(o.out.join(key), fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
        }
        openssl(
            &o,
            &[
                "verify",
                "-CAfile",
                "internal-root-ca.crt",
                "-purpose",
                "sslserver",
                "-verify_hostname",
                &o.hostname,
                "staging.crt",
            ],
            None,
        )
    })();
    result.map_err(|e| {
        format!(
            "{e}. Partial output remains in {}; use a new directory when retrying",
            o.out.display()
        )
    })?;
    println!("Generated and verified certificates in {}", o.out.display());
    println!(
        "CA: internal-root-ca.key (encrypted), internal-root-ca.crt\nServer: staging.key, staging.crt\nSupporting files: staging.csr, server.ext, internal-root-ca.srl"
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse(env::args().skip(1)).and_then(|o| match o {
        Some(o) => generate(o),
        None => {
            println!("{HELP}");
            Ok(())
        }
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rscert: {e}\nRun rscert --help for usage.");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_dns_names() {
        for good in ["staging.service.cloud", "localhost", "a-b.example"] {
            assert!(valid_hostname(good));
        }
        for bad in [
            "",
            "-bad.example",
            "bad-.example",
            "a..b",
            "*.example",
            "127.0.0.1",
            "x\nkeyUsage=anything",
            "a/b",
        ] {
            assert!(!valid_hostname(bad));
        }
    }
    #[test]
    fn rejects_invalid_validity_and_subjects() {
        for args in [
            vec!["--hostname", "a", "--server-days", "0"],
            vec!["--hostname", "a", "--ca-days", "1"],
            vec!["--hostname", "a", "--ca-name", "CA/O=Injected"],
            vec!["--hostname"],
            vec!["--unknown", "x"],
        ] {
            assert!(parse(args.into_iter().map(str::to_owned)).is_err());
        }
    }
}
