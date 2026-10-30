//! `rootstock status`: current version, latest available version and whether it is a major bump.

use std::path::Path;

use anyhow::{Result, bail};
use regex::Regex;
use semver::Version;

use crate::record;
use crate::source::{latest, parse_source};

/// What `status` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Version as recorded in the project.
    pub current: String,
    /// Tag of the latest release.
    pub latest: String,
    /// True when the latest major is above the current one.
    pub major: bool,
}

/// Parses a tag as a release version; a `git describe` suffix `-N-g<hex>` is ignored.
fn parse_release(tag: &str) -> Option<Version> {
    let rest = tag.strip_prefix('v')?;
    let describe = Regex::new(r"^(.*?)-\d+-g[0-9a-f]+$").ok()?;
    let base = describe
        .captures(rest)
        .map_or(rest, |c| c.get(1).map_or(rest, |m| m.as_str()));
    Version::parse(base).ok()
}

pub fn run(project: &Path) -> Result<Status> {
    let rec = record::read(project)?;
    let src = parse_source(&rec.source)?;
    let latest_tag = latest(&src)?;
    let Some(current) = parse_release(&rec.version) else {
        bail!("current version `{}` is not a release", rec.version);
    };
    let Some(newest) = parse_release(&latest_tag) else {
        bail!("latest tag `{latest_tag}` is not a release");
    };
    Ok(Status {
        major: newest.major > current.major,
        current: rec.version,
        latest: latest_tag,
    })
}
