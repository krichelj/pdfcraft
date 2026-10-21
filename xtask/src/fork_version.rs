//! The fork's own `cargo xtask version` subcommands, kept out of version.rs (upstream's file) so an
//! upstream change there can't conflict with them: `base`, `fork [tag]`, `bump-fork [tag]` (the
//! `<upstream>-p<N>` line, CLAUDE.md) and `check-upstream [vX.Y.Z]` (the gate on it).

use std::path::Path;

use crate::version::{read, validate};

/// The upstream base version without any fork or pre-release suffix (e.g. `0.6.0` from `0.6.0-p1`).
pub fn base_version(v: &str) -> &str {
    v.split_once('-').map(|(base, _)| base).unwrap_or(v)
}

/// Construct a fork patchlevel version from the current version and tag prefix (default `"p"`).
pub fn fork_version(current: &str, tag_prefix: &str) -> Result<String, String> {
    if tag_prefix.is_empty() || !tag_prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(format!("`{tag_prefix}` is not a valid tag prefix (must be alphanumeric or '-')"));
    }
    let base = base_version(current);
    let candidate = format!("{base}-{tag_prefix}1");
    validate(&candidate)?;
    Ok(candidate)
}

/// Bump the fork revision (e.g. `0.6.0-p1` -> `0.6.0-p2`).
pub fn bump_fork(current: &str, tag_prefix: &str) -> Result<String, String> {
    if tag_prefix.is_empty() || !tag_prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(format!("`{tag_prefix}` is not a valid tag prefix"));
    }
    let base = base_version(current);
    let prefix = format!("{base}-{tag_prefix}");
    if let Some(rev_str) = current.strip_prefix(&prefix)
        && let Ok(rev) = rev_str.parse::<u64>()
    {
        let next = rev.checked_add(1).ok_or_else(|| format!("`{current}`: patchlevel overflows"))?;
        let candidate = format!("{prefix}{next}");
        validate(&candidate)?;
        return Ok(candidate);
    }
    let candidate = format!("{base}-{tag_prefix}1");
    validate(&candidate)?;
    Ok(candidate)
}

/// storytold/pdfcraft's latest release, whose tag its releases page redirects to (no API quota).
pub const UPSTREAM_LATEST: &str = "https://github.com/storytold/pdfcraft/releases/latest";

/// (major, minor, patch) of `0.9.0`, `v0.9.0` or `0.9.0-p3`; `None` for other shapes.
fn core(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split(['-', '+']).next()?;
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    let c = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(c)
}

/// The gate on the fork's version line (CLAUDE.md): `fork` must be `<X.Y.Z>-p<N>` (N >= 1) built on
/// top of upstream's releases, so X.Y.Z is at least `upstream`, storytold/pdfcraft's latest release.
pub fn check_upstream(fork: &str, upstream: &str) -> Result<(), String> {
    let (base, level) = fork.split_once("-p").ok_or_else(|| format!("{fork} is not a fork version <X.Y.Z>-p<N>"))?;
    let fork_core = core(base).filter(|_| !base.contains('-')).ok_or_else(|| format!("{fork} is not a fork version <X.Y.Z>-p<N>"))?;
    let n: u64 = level.parse().map_err(|_| format!("{fork}: -p{level} is not a patchlevel"))?;
    if n == 0 {
        return Err(format!("{fork}: patchlevels start at -p1"));
    }
    let up = core(upstream).ok_or_else(|| format!("upstream release {upstream} is not a version"))?;
    if fork_core < up {
        return Err(format!(
            "{fork} is built on {base}, older than upstream's release {}: sync upstream (Sync upstream workflow) so the fork is {}-p1 or later",
            upstream.trim_start_matches('v'),
            upstream.trim_start_matches('v')
        ));
    }
    println!("version: {fork} is on top of upstream's {upstream}");
    Ok(())
}

/// storytold/pdfcraft's latest release tag, asked of GitHub with curl.
fn latest_upstream_release() -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSI", "-o", "/dev/null", "-w", "%{redirect_url}", UPSTREAM_LATEST])
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    url.rsplit_once("/tag/").map(|(_, t)| t.to_string()).filter(|t| core(t).is_some()).ok_or_else(|| format!("no upstream release found ({url})"))
}

/// The fork's subcommands, or `None` for one of version.rs's own.
pub fn run(root: &Path, args: &[&str]) -> Option<Result<(), String>> {
    let text = || std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("read Cargo.toml: {e}"));
    let set = |new: Result<String, String>| new.and_then(|v| crate::version::run(root, &["set", &v]));
    Some(match args {
        ["base"] => text().and_then(|t| read(&t)).map(|v| println!("{}", base_version(&v))),
        ["fork"] => set(text().and_then(|t| read(&t)).and_then(|v| fork_version(&v, "p"))),
        ["fork", tag] => set(text().and_then(|t| read(&t)).and_then(|v| fork_version(&v, tag))),
        ["bump-fork"] => set(text().and_then(|t| read(&t)).and_then(|v| bump_fork(&v, "p"))),
        ["bump-fork", tag] => set(text().and_then(|t| read(&t)).and_then(|v| bump_fork(&v, tag))),
        ["check-upstream"] => text().and_then(|t| read(&t)).and_then(|v| check_upstream(&v, &latest_upstream_release()?)),
        ["check-upstream", tag] => text().and_then(|t| read(&t)).and_then(|v| check_upstream(&v, tag)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fork_version_must_sit_on_top_of_upstreams_latest_release() {
        check_upstream("0.9.0-p1", "v0.9.0").unwrap();
        check_upstream("0.9.0-p7", "v0.9.0").unwrap();
        check_upstream("0.9.1-p1", "v0.9.0").unwrap();
        check_upstream("1.0.0-p2", "0.9.9").unwrap();
        let behind = check_upstream("0.8.0-p9", "v0.9.0").unwrap_err();
        assert!(behind.contains("older than upstream's release 0.9.0") && behind.contains("0.9.0-p1"), "{behind}");
        for bad in ["0.9.0", "0.9.0-p0", "0.9.0-rc.1", "0.9.0-px", "0.9-p1", "0.9.0-rc-p1"] {
            assert!(check_upstream(bad, "v0.9.0").is_err(), "{bad}");
        }
        assert!(check_upstream("0.9.0-p1", "nightly").is_err());
    }
    #[test]
    fn computes_base_and_fork_versions() {
        assert_eq!(base_version("0.6.0"), "0.6.0");
        assert_eq!(base_version("0.6.0-p1"), "0.6.0");
        assert_eq!(base_version("1.2.3-rc.2"), "1.2.3");

        assert_eq!(fork_version("0.6.0", "p").unwrap(), "0.6.0-p1");
        assert_eq!(fork_version("0.6.0-p5", "p").unwrap(), "0.6.0-p1");
    }
    #[test]
    fn bumps_fork_version() {
        assert_eq!(bump_fork("0.6.0", "p").unwrap(), "0.6.0-p1");
        assert_eq!(bump_fork("0.6.0-p1", "p").unwrap(), "0.6.0-p2");
        assert_eq!(bump_fork("0.6.0-p9", "p").unwrap(), "0.6.0-p10");
        assert_eq!(bump_fork("0.6.0-upstream.1", "p").unwrap(), "0.6.0-p1");
    }
}
