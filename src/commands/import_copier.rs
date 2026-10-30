//! `rootstock import-copier`: converts a Copier template in a directory into `rootstock.toml`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::config::CONFIG_FILE;
use crate::copier::{self, CopierError};

/// Permissions of the written `rootstock.toml`.
#[cfg(unix)]
const CONFIG_MODE: u32 = 0o644;

const COPIER_FILES: [&str; 2] = ["copier.yml", "copier.yaml"];

/// Converts the template in `dir`; the summary and any findings go to `err`.
///
/// Nothing is written unless the whole conversion succeeded. The answers-file
/// template is removed last, after `rootstock.toml` is in place.
pub fn run(dir: &Path, err: &mut dyn Write) -> Result<()> {
    let mut present: Vec<&str> = Vec::new();
    for name in COPIER_FILES {
        let path = dir.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_file() => present.push(name),
            // The template is untrusted: a link could pull in any file on the machine.
            Ok(_) => bail!("{name} must be a regular file"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow!(e).context(format!("cannot inspect {name}"))),
        }
    }
    let copier_file = match present.as_slice() {
        [one] => *one,
        [] => bail!("no copier.yml here (looked for copier.yml and copier.yaml)"),
        _ => bail!("both copier.yml and copier.yaml exist; keep only one"),
    };
    let target = dir.join(CONFIG_FILE);
    if std::fs::symlink_metadata(&target).is_ok() {
        bail!("{CONFIG_FILE} already exists; nothing was changed");
    }
    let yaml = std::fs::read_to_string(dir.join(copier_file))
        .with_context(|| format!("cannot read {copier_file}"))?;
    let converted = match copier::convert(&yaml, dir) {
        Ok(converted) => converted,
        Err(CopierError::Unsupported(findings)) => {
            // Best effort: the error below still ends the run with code 1.
            let _ = writeln!(err, "{copier_file} uses features rootstock cannot convert:");
            for finding in &findings {
                let _ = writeln!(err, "  {finding}");
            }
            bail!(
                "{} unsupported feature{} found; nothing was written",
                findings.len(),
                if findings.len() == 1 { "" } else { "s" }
            );
        }
        Err(e) => return Err(anyhow!(e).context(format!("cannot convert {copier_file}"))),
    };

    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("cannot create a temporary file in {}", dir.display()))?;
    tmp.write_all(converted.text.as_bytes())
        .context("cannot write the temporary file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The temporary file is created 0600; the config is an ordinary repository file.
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(CONFIG_MODE))
            .context("cannot set the permissions of the temporary file")?;
    }
    tmp.persist_noclobber(&target)
        .map_err(|e| anyhow!(e.error))
        .with_context(|| format!("cannot write {CONFIG_FILE}"))?;
    let _ = writeln!(err, "wrote {CONFIG_FILE}");

    for rel in &converted.remove {
        std::fs::remove_file(dir.join(rel)).with_context(|| {
            format!("{CONFIG_FILE} is already written, but cannot remove {rel}")
        })?;
        let _ = writeln!(err, "removed {rel}");
    }
    Ok(())
}
