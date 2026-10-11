//! `cargo xtask version [base | set X.Y.Z[-pre] | fork [tag] | bump-fork [tag]]`: workspace versioning.
//!
//! The version lives in `[workspace.package] version` in the root `Cargo.toml`; every crate
//! inherits it with `version.workspace = true`, and the packaging scripts read it from here.
//! Internal path dependencies in `[workspace.dependencies]` also have their `version = "…"`
//! requirements kept strictly in sync, ensuring `cargo update --workspace` resolves cleanly.
//!
//! For downstream forks, an orthogonal SemVer-compliant patchlevel extension scheme is used:
//! `<base>-p<N>` (e.g. `0.6.0-p1`, `0.6.0-p2`).

use std::path::Path;

/// The `version = "…"` value inside `[workspace.package]`.
pub fn read(manifest: &str) -> Result<String, String> {
    let (_, line) = find_line(manifest)?;
    let v = line.split_once('=').map(|(_, v)| v.trim().trim_matches('"').to_string()).unwrap_or_default();
    if v.is_empty() {
        return Err("empty [workspace.package] version".into());
    }
    Ok(v)
}

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
    if let Some(rev_str) = current.strip_prefix(&prefix) {
        if let Ok(rev) = rev_str.parse::<u64>() {
            let candidate = format!("{prefix}{}", rev + 1);
            validate(&candidate)?;
            return Ok(candidate);
        }
    }
    let candidate = format!("{base}-{tag_prefix}1");
    validate(&candidate)?;
    Ok(candidate)
}

/// `manifest` with the workspace version replaced; everything else is byte-for-byte unchanged.
///
/// Updates both `[workspace.package] version` and all internal `pdfcraft-*` path-dependency
/// version pins in `[workspace.dependencies]`.
pub fn replace(manifest: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let (idx, _) = find_line(manifest)?;
    let mut out = String::with_capacity(manifest.len() + 16);
    let mut in_deps = false;
    for (i, line) in manifest.split_inclusive('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[workspace.dependencies]";
        }
        if i == idx {
            let eol = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version = \"{new}\"{eol}"));
        } else if in_deps && trimmed.starts_with("pdfcraft-") && trimmed.contains("path =") && trimmed.contains("version = \"") {
            if let Some(pos) = line.find("version = \"") {
                let start = pos + "version = \"".len();
                if let Some(end) = line[start..].find('"') {
                    let mut modified = String::with_capacity(line.len() + new.len());
                    modified.push_str(&line[..start]);
                    modified.push_str(new);
                    modified.push_str(&line[start + end..]);
                    out.push_str(&modified);
                    continue;
                }
            }
            out.push_str(line);
        } else {
            out.push_str(line);
        }
    }
    Ok(out)
}

/// Semver without build metadata: `MAJOR.MINOR.PATCH` plus an optional `-pre.release` tag.
pub fn validate(v: &str) -> Result<(), String> {
    let bad = || Err(format!("`{v}` is not a version like 1.2.3 or 1.2.3-rc.1"));
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    if nums.len() != 3 || nums.iter().any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || (n.len() > 1 && n.starts_with('0'))) {
        return bad();
    }
    if let Some(p) = pre
        && (p.is_empty() || p.split('.').any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')))
    {
        return bad();
    }
    Ok(())
}

/// Index and text of the `version` line in `[workspace.package]`.
fn find_line(manifest: &str) -> Result<(usize, &str), String> {
    let mut in_section = false;
    for (i, line) in manifest.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == "[workspace.package]";
            continue;
        }
        if in_section
            && let Some(rest) = t.strip_prefix("version")
            && rest.trim_start().starts_with('=')
        {
            return Ok((i, line));
        }
    }
    Err("no `version = \"…\"` in [workspace.package] of the root Cargo.toml".into())
}

fn set_version(root: &Path, path: &Path, text: &str, new: &str) -> Result<(), String> {
    let old = read(text)?;
    let updated = replace(text, new)?;
    let tmp = root.join("target").join("Cargo.toml.xtask-version");
    std::fs::create_dir_all(root.join("target")).map_err(|e| format!("create target/: {e}"))?;
    std::fs::write(&tmp, updated).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))?;
    let cargo_update = |extra: &[&str]| -> Result<(), String> {
        let st = std::process::Command::new(env!("CARGO"))
            .current_dir(root)
            .args(["update", "--workspace"])
            .args(extra)
            .status()
            .map_err(|e| format!("cargo update: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("cargo update --workspace {} failed", extra.join(" "))) }
    };
    if cargo_update(&["--offline"]).is_err() {
        cargo_update(&[])?;
    }
    println!("version: {old} -> {new}");
    Ok(())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match args {
        [] => {
            println!("{}", read(&text)?);
            Ok(())
        }
        ["base"] => {
            let current = read(&text)?;
            println!("{}", base_version(&current));
            Ok(())
        }
        ["fork"] => {
            let current = read(&text)?;
            let new = fork_version(&current, "p")?;
            set_version(root, &path, &text, &new)
        }
        ["fork", tag_prefix] => {
            let current = read(&text)?;
            let new = fork_version(&current, tag_prefix)?;
            set_version(root, &path, &text, &new)
        }
        ["bump-fork"] => {
            let current = read(&text)?;
            let new = bump_fork(&current, "p")?;
            set_version(root, &path, &text, &new)
        }
        ["bump-fork", tag_prefix] => {
            let current = read(&text)?;
            let new = bump_fork(&current, tag_prefix)?;
            set_version(root, &path, &text, &new)
        }
        ["set", new] => {
            set_version(root, &path, &text, new)
        }
        _ => Err("usage: cargo xtask version [base | set X.Y.Z[-pre] | fork [tag] | bump-fork [tag]]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.dependencies]\nfoo = { version = \"1\" }\n";

    #[test]
    fn reads_the_workspace_version() {
        assert_eq!(read(MANIFEST).unwrap(), "0.1.0");
        assert!(read("[package]\nversion = \"1.0.0\"\n").is_err());
    }

    #[test]
    fn replaces_only_the_workspace_version() {
        let out = replace(MANIFEST, "1.2.3-rc.1").unwrap();
        assert_eq!(read(&out).unwrap(), "1.2.3-rc.1");
        assert_eq!(out, MANIFEST.replace("version = \"0.1.0\"", "version = \"1.2.3-rc.1\""));
        assert!(out.contains("foo = { version = \"1\" }"));
    }

    #[test]
    fn replaces_workspace_and_internal_dependencies() {
        let manifest = "[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\nfoo = { version = \"1\" }\npdfcraft-geom = { path = \"crates/geom\", version = \"0.1.0\" }\n";
        let out = replace(manifest, "0.2.0-p1").unwrap();
        assert!(out.contains("version = \"0.2.0-p1\""));
        assert!(out.contains("pdfcraft-geom = { path = \"crates/geom\", version = \"0.2.0-p1\" }"));
        assert!(out.contains("foo = { version = \"1\" }"));
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = MANIFEST.replace('\n', "\r\n");
        let out = replace(&crlf, "2.0.0").unwrap();
        assert_eq!(out, crlf.replace("\"0.1.0\"", "\"2.0.0\""));
    }

    #[test]
    fn validates_versions() {
        for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2", "0.6.0-p1", "0.6.0-p2"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        assert!(replace(MANIFEST, "nope").is_err());
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

    #[test]
    fn the_real_manifest_has_a_valid_version() {
        let text = std::fs::read_to_string(crate::gates::root().join("Cargo.toml")).unwrap();
        validate(&read(&text).unwrap()).unwrap();
    }
}
