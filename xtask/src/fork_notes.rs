//! `cargo xtask fork-notes generate|check|body <version> | pending`: the record every krichelj/pdfcraft
//! release (`<base>-p<N>`, CLAUDE.md fork rules) carries in `docs/fork/releases/v<version>.md`.
//!
//! A record has two parts. "What changed" is written by hand and says what the release means for
//! people using it. "Based on" is generated from git and is the bookkeeping:
//! the storytold/pdfcraft commit and version the release is based on, and the previous fork release.
//! `check` regenerates the second part and refuses the release when the file is missing, the summary
//! is empty or still a TODO, or the generated part no longer matches git (an upstream merge since).
//! The Release workflow runs `body` (a `check`, then the release page's text) before it builds; CI
//! runs `pending` on every change, which checks the record of a version not released yet.
//!
//! Options: `--commit REV` (default HEAD), `--upstream REV` (default: storytold/pdfcraft's main,
//! fetched into `refs/fork-notes/upstream`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail, ensure};

use crate::version;

const UPSTREAM_URL: &str = "https://github.com/storytold/pdfcraft.git";
const UPSTREAM_REF: &str = "refs/fork-notes/upstream";
const UPSTREAM_WEB: &str = "https://github.com/storytold/pdfcraft";
const FORK_WEB: &str = "https://github.com/krichelj/pdfcraft";
const START: &str = "<!-- fork-notes:generated:start (cargo xtask fork-notes generate; do not edit by hand) -->";
const END: &str = "<!-- fork-notes:generated:end -->";
const SUMMARY: &str = "## What changed";
const PLACEHOLDER: &str = "TODO";
/// On every release page, between the summary and the generated part.
const INSTALL: &str = r#"## Install on a Mac with Apple silicon

Run this in Terminal. It downloads this release, checks it against `SHA256SUMS.txt`, and puts PdfCraft in Applications; run it again to update. From 0.8.0-p2 on, Help > Check for updates > Install and relaunch does the same from inside the app.

```sh
curl -fsSL https://raw.githubusercontent.com/krichelj/pdfcraft/main/packaging/macos/install.sh | bash
```

The DMG works too, but these builds aren't notarized by Apple, so macOS blocks a copy downloaded in a browser ("Apple could not verify PdfCraft"). To open it anyway, use System Settings > Privacy & Security > Open Anyway, or run `xattr -dr com.apple.quarantine /Applications/PdfCraft.app`.

"#;

/// Where the record for `version` lives.
pub fn notes_path(repo: &Path, version: &str) -> PathBuf {
    repo.join("docs").join("fork").join("releases").join(format!("v{version}.md"))
}

/// What git says one fork release is, relative to upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    pub version: String,
    pub upstream_base: String,
    pub upstream_version: String,
    /// The latest earlier fork release tag (`v<version>`) the commit contains.
    pub previous: Option<String>,
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let repo = crate::gates::root();
    let flag = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());
    let positional: Vec<&String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in args {
            if skip {
                skip = false;
            } else if a == "--commit" || a == "--upstream" {
                skip = true;
            } else {
                out.push(a);
            }
        }
        out
    };
    let commit = flag("--commit").unwrap_or_else(|| "HEAD".into());
    let usage = "usage: cargo xtask fork-notes generate <version> | check <version> | body <version> | pending [--commit REV] [--upstream REV]";
    let upstream = || -> anyhow::Result<String> {
        match flag("--upstream") {
            Some(rev) => Ok(rev),
            None => {
                git(&repo, &["fetch", "--quiet", UPSTREAM_URL, &format!("+refs/heads/main:{UPSTREAM_REF}")])
                    .context("fetching storytold/pdfcraft main")?;
                Ok(UPSTREAM_REF.into())
            }
        }
    };
    match positional.as_slice() {
        [cmd, v] if cmd.as_str() == "generate" => {
            let delta = collect(&repo, &normalize(v)?, &commit, &upstream()?)?;
            let path = generate(&repo, &delta)?;
            println!(
                "fork-notes: wrote {} (upstream {} at {}); write \"{SUMMARY}\" before releasing",
                path.display(),
                delta.upstream_version,
                short(&delta.upstream_base)
            );
            Ok(())
        }
        [cmd, v] if cmd.as_str() == "check" => {
            let delta = collect(&repo, &normalize(v)?, &commit, &upstream()?)?;
            check(&repo, &delta)?;
            println!("fork-notes: {} is ready for v{}", notes_path(&repo, &delta.version).display(), delta.version);
            Ok(())
        }
        [cmd, v] if cmd.as_str() == "body" => {
            let delta = collect(&repo, &normalize(v)?, &commit, &upstream()?)?;
            check(&repo, &delta)?;
            let text = std::fs::read_to_string(notes_path(&repo, &delta.version))?;
            print!("{}", body(&text)?);
            Ok(())
        }
        [cmd] if cmd.as_str() == "pending" => {
            let manifest = std::fs::read_to_string(repo.join("Cargo.toml")).context("reading Cargo.toml")?;
            let v = version::read(&manifest).map_err(anyhow::Error::msg)?;
            if !is_fork_version(&v) {
                println!("fork-notes: {v} is not a fork release (<base>-p<N>); nothing to check");
                return Ok(());
            }
            if git(&repo, &["rev-parse", "--verify", "--quiet", &format!("refs/tags/v{v}")]).is_ok() {
                println!("fork-notes: v{v} is already released; nothing to check until the next bump");
                return Ok(());
            }
            let delta = collect(&repo, &v, &commit, &upstream()?)?;
            check(&repo, &delta)?;
            println!("fork-notes: v{v} (not released yet) has its notes");
            Ok(())
        }
        _ => bail!("{usage}"),
    }
}

fn normalize(v: &str) -> anyhow::Result<String> {
    let v = v.trim().trim_start_matches('v').to_string();
    version::validate(&v).map_err(anyhow::Error::msg)?;
    Ok(v)
}

/// `<base>-p<N>`, the fork's patchlevel (CLAUDE.md).
fn is_fork_version(v: &str) -> bool {
    key(v).is_some_and(|k| k.3 > 0)
}

/// (major, minor, patch, fork patchlevel) of `0.8.0-p2` or `v0.8.0-p2`; `None` for other shapes.
fn key(v: &str) -> Option<(u64, u64, u64, u64)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let (core, level) = match v.split_once('-') {
        Some((core, pre)) => (core, pre.strip_prefix('p').filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()?),
        None => (v, 0),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let k = (parts.next()??, parts.next()??, parts.next()??, level);
    parts.next().is_none().then_some(k)
}

fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().context("running git")?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    String::from_utf8(out.stdout).context("git printed something that isn't UTF-8")
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// Read the facts for release `version` at `commit` from git.
pub fn collect(repo: &Path, version: &str, commit: &str, upstream: &str) -> anyhow::Result<Delta> {
    let commit = git(repo, &["rev-parse", "--verify", &format!("{commit}^{{commit}}")])?.trim().to_string();
    let upstream = git(repo, &["rev-parse", "--verify", &format!("{upstream}^{{commit}}")])?.trim().to_string();
    let upstream_base = git(repo, &["merge-base", &commit, &upstream]).context("the release shares no history with upstream")?.trim().to_string();
    let base_manifest = git(repo, &["show", &format!("{upstream_base}:Cargo.toml")])?;
    let upstream_version = version::read(&base_manifest).map_err(anyhow::Error::msg)?;
    let this = key(version).with_context(|| format!("{version} is not a <base>-p<N> fork version"))?;
    let previous = git(repo, &["tag", "--merged", &commit, "--list", "v*"])?
        .lines()
        .filter_map(|t| key(t).filter(|k| *k < this && k.3 > 0).map(|k| (k, t.to_string())))
        .max()
        .map(|(_, t)| t);
    Ok(Delta { version: version.to_string(), upstream_base, upstream_version, previous })
}

/// The generated part of the record, markers included.
pub fn render(d: &Delta) -> String {
    let mut s = format!("{START}\n## Based on\n\n");
    s += &format!(
        "- **Upstream base:** [storytold/pdfcraft@{}]({UPSTREAM_WEB}/commit/{}), upstream version {}.\n",
        short(&d.upstream_base),
        d.upstream_base,
        d.upstream_version
    );
    match &d.previous {
        Some(p) => s += &format!("- **Previous fork release:** [{p}]({FORK_WEB}/releases/tag/{p}).\n"),
        None => s += "- **Previous fork release:** none; this is the first published fork release.\n",
    }
    s += END;
    s += "\n";
    s
}

/// A new record: the title, an unwritten summary, and the generated part.
fn template(d: &Delta) -> String {
    format!(
        "# PdfCraft {}\n\n{SUMMARY}\n\n- {PLACEHOLDER}: what this release changes for people using it, one line per change.\n\n{}",
        d.version,
        render(d)
    )
}

/// Write the record, or refresh the generated part of an existing one (the summary is kept).
pub fn generate(repo: &Path, d: &Delta) -> anyhow::Result<PathBuf> {
    let path = notes_path(repo, &d.version);
    let text = match std::fs::read_to_string(&path) {
        Ok(old) => {
            let (before, _, after) = split(&old)?;
            format!("{before}{}{}", render(d), after.trim_start_matches('\n'))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => template(d),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// (text before the generated part, the generated part with markers, text after it).
fn split(text: &str) -> anyhow::Result<(&str, &str, &str)> {
    ensure!(text.matches(START).count() == 1 && text.matches(END).count() == 1, "the generated part's markers are missing or repeated");
    let a = text.find(START).context("no start marker")?;
    let b = text.find(END).context("no end marker")?;
    ensure!(a < b, "the generated part's markers are out of order");
    let end = b + END.len();
    let end = if text.get(end..).is_some_and(|rest| rest.starts_with('\n')) { end + 1 } else { end };
    match (text.get(..a), text.get(a..end), text.get(end..)) {
        (Some(before), Some(generated), Some(after)) => Ok((before, generated, after)),
        _ => bail!("the generated part's markers are malformed"),
    }
}

/// A release page's text from a record: the summary, how to install, the generated part (the
/// page already shows the title).
pub fn body(record: &str) -> anyhow::Result<String> {
    let (before, generated, after) = split(record)?;
    let summary = before.split_once('\n').map_or("", |(_, rest)| rest).trim();
    Ok(format!("{summary}\n\n{INSTALL}{generated}{after}"))
}

/// Why the record for `d` can't ship, if it can't.
pub fn check(repo: &Path, d: &Delta) -> anyhow::Result<()> {
    let path = notes_path(repo, &d.version);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} is missing: run `cargo xtask fork-notes generate {}` and write \"{SUMMARY}\"", path.display(), d.version))?;
    check_text(&text, d).with_context(|| format!("{} can't ship", path.display()))
}

fn check_text(text: &str, d: &Delta) -> anyhow::Result<()> {
    let title = format!("# PdfCraft {}", d.version);
    ensure!(text.lines().next().is_some_and(|l| l.trim() == title), "its first line must be `{title}`");
    let (before, generated, _) = split(text)?;
    let summary = before.split_once(SUMMARY).map(|(_, rest)| rest).with_context(|| format!("it has no \"{SUMMARY}\" section"))?;
    ensure!(!summary.contains(PLACEHOLDER), "\"{SUMMARY}\" still has a {PLACEHOLDER}: say what this release changes");
    let items = summary.lines().filter(|l| l.trim_start().starts_with("- ") && l.trim().len() > 2).count();
    ensure!(items > 0, "\"{SUMMARY}\" lists no changes (one `- ` line per change)");
    ensure!(
        generated == render(d),
        "the generated part doesn't match git (an upstream merge or a new release since): run `cargo xtask fork-notes generate {}`",
        d.version
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta() -> Delta {
        Delta { version: "0.8.0-p2".into(), upstream_base: "2".repeat(40), upstream_version: "0.8.0".into(), previous: Some("v0.8.0-p1".into()) }
    }

    fn written(d: &Delta, summary: &str) -> String {
        template(d).replace(&format!("- {PLACEHOLDER}: what this release changes for people using it, one line per change."), summary)
    }

    #[test]
    fn the_release_page_has_the_summary_then_install_then_the_differences() {
        let d = delta();
        let b = body(&written(&d, "- One change.")).unwrap();
        assert!(!b.contains("# PdfCraft 0.8.0-p2"), "the page shows the title already");
        let (sum, inst, diff) = (b.find("## What changed").unwrap(), b.find("## Install").unwrap(), b.find(START).unwrap());
        assert!(sum < inst && inst < diff, "{b}");
        assert!(b.contains("- One change.") && b.contains("install.sh | bash"));
    }

    #[test]
    fn versions_order_by_number_and_patchlevel() {
        assert_eq!(key("v0.8.0-p2"), Some((0, 8, 0, 2)));
        assert_eq!(key("0.8.0"), Some((0, 8, 0, 0)));
        assert!(key("v0.8.0-p10") > key("v0.8.0-p9"));
        assert!(key("v0.9.0-p1") > key("v0.8.0-p10"));
        for bad in ["v0.8.0-rc.1", "v0.8", "v0.8.0.1", "nightly", "v0.8.0-p", "v0.8.0-px"] {
            assert_eq!(key(bad), None, "{bad}");
        }
        assert!(is_fork_version("0.8.0-p1") && !is_fork_version("0.8.0"));
    }

    #[test]
    fn the_generated_part_names_the_upstream_base_and_the_previous_release() {
        let r = render(&delta());
        assert!(r.starts_with(START) && r.trim_end().ends_with(END));
        assert!(r.contains("storytold/pdfcraft@222222222222"), "{r}");
        assert!(r.contains("upstream version 0.8.0"));
        assert!(r.contains("[v0.8.0-p1](https://github.com/krichelj/pdfcraft/releases/tag/v0.8.0-p1)"));
        assert!(!r.contains("###"), "no commit or file tables: {r}");
        let first = Delta { previous: None, ..delta() };
        assert!(render(&first).contains("none; this is the first published fork release"));
    }

    #[test]
    fn a_written_record_passes_and_every_shortcut_is_refused() {
        let d = delta();
        let good = written(&d, "- Check for updates installs the new version.");
        check_text(&good, &d).unwrap();
        // The summary is what a person writes: refused while it's the placeholder or empty.
        assert!(check_text(&template(&d), &d).unwrap_err().to_string().contains(PLACEHOLDER));
        assert!(check_text(&written(&d, ""), &d).unwrap_err().to_string().contains("lists no changes"));
        assert!(check_text(&written(&d, "Some prose but no list."), &d).is_err());
        assert!(check_text(&good.replace(SUMMARY, "## Changes"), &d).is_err(), "the section is required by name");
        assert!(check_text(&good.replace("# PdfCraft 0.8.0-p2", "# PdfCraft 0.8.0-p1"), &d).is_err(), "the title names the version");
        // The generated part must be what git says now.
        let rebased = Delta { upstream_base: "4".repeat(40), ..d.clone() };
        assert!(check_text(&good, &rebased).unwrap_err().to_string().contains("doesn't match git"));
        let edited = good.replace("upstream version 0.8.0.", "upstream version 0.7.0.");
        assert!(check_text(&edited, &d).is_err(), "hand edits to the generated part are refused");
        assert!(check_text(&good.replace(END, ""), &d).is_err(), "markers are required");
    }

    /// A throwaway repository: an upstream line, a fork with a released `v0.1.0-p1`, and more.
    struct Repo(PathBuf);
    impl Repo {
        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(&self.0)
                .args(["-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap()
        }
        fn commit(&self, file: &str, body: &str, msg: &str) {
            std::fs::write(self.0.join(file), body).unwrap();
            self.git(&["add", file]);
            self.git(&["commit", "-q", "-m", msg]);
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn repo(name: &str) -> Repo {
        let dir = std::env::temp_dir().join(format!("pdfcraft-fork-notes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Repo(dir);
        r.git(&["init", "-q", "-b", "main"]);
        r.commit("Cargo.toml", "[workspace.package]\nversion = \"0.1.0\"\n", "upstream: 0.1.0");
        r.commit("up.rs", "1", "upstream: feature");
        r.git(&["branch", "upstream"]);
        r.commit("Cargo.toml", "[workspace.package]\nversion = \"0.1.0-p1\"\n", "Fork: 0.1.0-p1");
        r.git(&["tag", "v0.1.0-p1"]);
        r.git(&["checkout", "-q", "upstream"]);
        r.commit("up.rs", "2", "upstream: later work");
        r.git(&["checkout", "-q", "main"]);
        r.git(&["merge", "-q", "--no-edit", "upstream"]);
        r.commit("fork.rs", "f", "Fork: installer");
        r
    }

    #[test]
    fn git_gives_the_upstream_base_and_the_previous_release() {
        let r = repo("collect");
        let d = collect(&r.0, "0.1.0-p2", "HEAD", "upstream").unwrap();
        assert_eq!(d.upstream_base, r.git(&["rev-parse", "upstream"]).trim(), "the upstream commit merged last");
        assert_eq!(d.upstream_version, "0.1.0");
        assert_eq!(d.previous.as_deref(), Some("v0.1.0-p1"));
        let first = collect(&r.0, "0.1.0-p1", "v0.1.0-p1", "upstream").unwrap();
        assert_eq!(first.previous, None);
        assert_ne!(first.upstream_base, d.upstream_base, "p1 came before the later upstream merge");
    }

    #[test]
    fn generate_then_check_and_an_upstream_merge_makes_it_stale() {
        let r = repo("roundtrip");
        let d = collect(&r.0, "0.1.0-p2", "HEAD", "upstream").unwrap();
        let path = generate(&r.0, &d).unwrap();
        assert!(check(&r.0, &d).is_err(), "a freshly generated record has no summary yet");
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text.replace(&format!("- {PLACEHOLDER}: what this release changes for people using it, one line per change."), "- The installer.");
        std::fs::write(&path, &text).unwrap();
        check(&r.0, &d).unwrap();
        // Committing the record, merging its branch and further fork commits leave it current.
        r.git(&["checkout", "-q", "-b", "notes"]);
        r.git(&["add", "docs"]);
        r.git(&["commit", "-q", "-m", "Fork: notes for 0.1.0-p2"]);
        r.git(&["checkout", "-q", "main"]);
        r.git(&["merge", "-q", "--no-ff", "--no-edit", "notes"]);
        r.commit("fork2.rs", "g", "Fork: another change");
        check(&r.0, &collect(&r.0, "0.1.0-p2", "HEAD", "upstream").unwrap()).unwrap();
        // Merging newer upstream work moves the base: the record must be regenerated.
        r.git(&["checkout", "-q", "upstream"]);
        r.commit("up.rs", "3", "upstream: even later");
        r.git(&["checkout", "-q", "main"]);
        r.git(&["merge", "-q", "--no-edit", "upstream"]);
        let merged = collect(&r.0, "0.1.0-p2", "HEAD", "upstream").unwrap();
        assert!(check(&r.0, &merged).is_err(), "an upstream merge after the notes were generated");
        generate(&r.0, &merged).unwrap();
        check(&r.0, &merged).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("- The installer."), "regenerating keeps the summary");
        assert!(check(&r.0, &collect(&r.0, "0.1.0-p3", "HEAD", "upstream").unwrap()).is_err(), "no record for another version");
    }
}
