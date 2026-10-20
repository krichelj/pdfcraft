//! Checks on the GitHub Actions workflows that only tests need: CI and CD use the host's cores in
//! parallel (the owner's rule, CLAUDE.md), so none of these may quietly fall back to serial.

#[cfg(test)]
mod tests {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const ALL: [(&str, &str); 7] = [
        ("ci.yml", CI),
        ("release.yml", include_str!("../../.github/workflows/release.yml")),
        ("packaging-lint.yml", include_str!("../../.github/workflows/packaging-lint.yml")),
        ("own-runners-guard.yml", include_str!("../../.github/workflows/own-runners-guard.yml")),
        ("sync-upstream.yml", include_str!("../../.github/workflows/sync-upstream.yml")),
        ("fork-automation.yml", include_str!("../../.github/workflows/fork-automation.yml")),
        ("auto-merge.yml", include_str!("../../.github/workflows/auto-merge.yml")),
    ];

    fn file(name: &str) -> &'static str {
        ALL.iter().find(|(f, _)| *f == name).map(|(_, t)| *t).unwrap_or_else(|| panic!("{name} is in ALL"))
    }

    /// The jobs of a workflow as (name, text), split at the two-space keys under `jobs:`.
    fn jobs(workflow: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut in_jobs = false;
        for line in workflow.lines() {
            if line == "jobs:" {
                in_jobs = true;
                continue;
            }
            if !in_jobs {
                continue;
            }
            let is_key = line.starts_with("  ") && !line.starts_with("   ") && !line.trim_start().starts_with('#') && line.trim_end().ends_with(':');
            if is_key {
                out.push((line.trim().trim_end_matches(':').to_string(), String::new()));
            } else if let Some((_, text)) = out.last_mut() {
                text.push_str(line);
                text.push('\n');
            }
        }
        out
    }

    /// The commands a job runs: its `run:` lines and their block bodies, comments dropped.
    fn commands(job: &str) -> Vec<&str> {
        job.lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.starts_with("- name:") && !l.starts_with("uses:")).collect()
    }

    #[test]
    fn tests_run_side_by_side_not_one_binary_at_a_time() {
        let test = jobs(CI).into_iter().find(|(name, _)| name == "test").expect("ci.yml has a test job").1;
        let cmds = commands(&test);
        assert!(cmds.iter().any(|c| c.starts_with("cargo nextest run --workspace")), "the tests run under nextest");
        assert!(cmds.iter().any(|c| c.starts_with("cargo test --workspace --doc")), "and the doctests, which nextest skips");
        // `cargo test` runs its test binaries one after another; only doctests may use it.
        for (file, text) in ALL {
            for c in commands(text).into_iter().filter(|c| c.contains("cargo test")) {
                assert!(c.contains("--doc"), "{file}: `{c}` runs the test binaries serially; use cargo nextest run");
            }
        }
    }

    #[test]
    fn every_job_that_builds_rust_saturates_the_host() {
        let building = |job: &str| {
            commands(job)
                .iter()
                .any(|c| ["cargo test", "cargo nextest", "cargo clippy", "cargo xtask", "cargo check", "cargo build"].iter().any(|b| c.contains(b)))
        };
        let mut checked = 0;
        for (name, job) in jobs(CI).into_iter().filter(|(_, job)| building(job)) {
            assert!(job.contains("Saturate host threads"), "ci.yml job `{name}` builds Rust without the saturate step");
            checked += 1;
        }
        assert!(checked >= 2, "found the test and fuzz jobs ({checked})");
    }

    #[test]
    fn builds_are_capped_where_more_jobs_stop_helping_not_below() {
        // 64 is where a clean CI build stopped getting faster (ci.yml says how it was measured).
        // A lower cap would make builds slower again; a missing one runs one job per thread.
        for (name, job) in jobs(CI).into_iter().filter(|(_, job)| job.contains("Saturate host threads")) {
            let cap = job
                .lines()
                .find_map(|l| l.trim().strip_prefix("BUILD_JOBS_CAP="))
                .unwrap_or_else(|| panic!("ci.yml job `{name}` sets no BUILD_JOBS_CAP"))
                .parse::<u32>()
                .unwrap_or_else(|e| panic!("ci.yml job `{name}`: BUILD_JOBS_CAP is not a number ({e})"));
            assert!(cap >= 64, "ci.yml job `{name}` caps builds at {cap} jobs, below the measured 64");
            assert!(job.contains("echo \"CARGO_BUILD_JOBS=${BUILD_JOBS}\""), "ci.yml job `{name}` builds with the capped count");
        }
    }

    #[test]
    fn a_release_is_refused_without_its_record() {
        // The record (docs/fork/releases) is checked before anything is built, and is the page.
        let release = ALL.iter().find(|(f, _)| *f == "release.yml").expect("release.yml").1;
        let version = jobs(release).into_iter().find(|(n, _)| n == "version").expect("a version job").1;
        assert!(version.contains("cargo xtask fork-notes body \"$V\" --commit \"$GITHUB_SHA\""), "the version job checks the record");
        assert!(version.contains("fetch-depth: 0"), "with the history and tags it needs");
        let draft = jobs(release).into_iter().find(|(n, _)| n == "release").expect("a release job").1;
        assert_eq!(draft.matches("--notes-file \"$GITHUB_WORKSPACE/notes/release-notes.md\"").count(), 2, "created and updated drafts use it");
        assert!(!draft.contains("--generate-notes"), "no notes but the record");
        let test = jobs(CI).into_iter().find(|(n, _)| n == "test").expect("ci.yml test job").1;
        assert!(test.contains("cargo xtask fork-notes pending"), "a bump's PR is checked too");
        assert!(test.contains("fetch-depth: 0"));
    }

    #[test]
    fn upstream_is_synced_daily_and_merged_through_a_checked_pr() {
        let sync = file("sync-upstream.yml");
        assert!(sync.contains("schedule:") && sync.contains("cron: \"17 * * * *\""), "every hour");
        assert!(sync.contains("cargo xtask version check-upstream"), "then proves the version is on top of upstream's release");
        let version = jobs(file("release.yml")).into_iter().find(|(n, _)| n == "version").expect("version job").1;
        assert!(version.contains("cargo xtask version check-upstream"), "and Release refuses a version that isn't");
        assert!(sync.contains("workflow_dispatch:"));
        assert!(sync.contains("run: packaging/fork/sync-upstream.sh"));
        assert!(sync.contains("persist-credentials: false"), "never pushes with the workflow's token");
        let script = include_str!("../../packaging/fork/sync-upstream.sh");
        let lib = include_str!("../../packaging/fork/lib.sh");
        assert!(script.contains("gh pr create") && script.contains("merge_when_green \"$pr\""));
        assert!(lib.contains("gh pr merge \"$pr\" --repo \"$REPO\" --merge --delete-branch"));
        assert!(lib.contains(".name == \"test (shared code)\" and .conclusion == \"success\""), "merges only after CI's tests passed");
        assert!(script.contains("cargo xtask fork-notes check"), "and with its release record");
        let guard = file("own-runners-guard.yml");
        assert!(guard.contains("\"Sync upstream\""), "the guard watches it");
        let scenarios = file("fork-automation.yml");
        assert!(
            scenarios.contains("packaging/fork/test-sync.sh") && scenarios.contains("\"packaging/fork/**\""),
            "its scenarios run on PRs that change it"
        );
    }

    #[test]
    fn a_push_to_main_publishes_an_unreleased_version_and_checks_its_dmg() {
        let release = file("release.yml");
        assert!(release.contains("push:\n    branches: [main]"), "releases run from main only");
        let version = jobs(release).into_iter().find(|(n, _)| n == "version").expect("a version job").1;
        assert!(version.contains("refs/tags/v$V") && version.contains("skip=true"), "a released version is skipped");
        for job in ["macos", "release"] {
            let j = jobs(release).into_iter().find(|(n, _)| n == job).expect(job).1;
            assert!(j.contains("if: needs.version.outputs.skip != 'true'"), "{job} skips released versions");
        }
        let bump = jobs(release).into_iter().find(|(n, _)| n == "bump").expect("a bump job").1;
        assert!(
            bump.contains("if: needs.version.outputs.skip == 'true' && github.event_name == 'push'"),
            "a released version that changed is bumped"
        );
        assert!(bump.contains("run: packaging/fork/release-bump.sh") && bump.contains("persist-credentials: false"));
        let script = include_str!("../../packaging/fork/release-bump.sh");
        assert!(script.contains("next=\"${current%%-*}-p$((level + 1))\""), "the fork's own -p<N> line, beside upstream's version");
        assert!(script.contains("merge_when_green"), "through a checked PR");
        let draft = jobs(release).into_iter().find(|(n, _)| n == "release").expect("release job").1;
        assert!(draft.contains("gh release edit \"v$VERSION\" --draft=false --latest"), "published, not left a draft");
        let check = draft.split("name: Check the published DMG").nth(1).expect("a published-DMG check");
        for must in ["gh release download", "sha256sum -c", "Mach-O 64-bit arm64 executable", "code_directory", " Applications$"] {
            assert!(check.contains(must), "the published DMG check covers {must}");
        }
    }

    #[test]
    fn each_check_runs_once_where_it_belongs() {
        // PRs carry the checks; only Release runs on a push (to main); the guard checks each other
        // run once, when it completes (the owner, 2026-10-16: no duplicate runs).
        for name in ["ci.yml", "packaging-lint.yml", "own-runners-guard.yml", "sync-upstream.yml", "fork-automation.yml"] {
            assert!(!file(name).contains("\n  push:"), "{name} runs on pushes too: its PR run already covers them");
        }
        assert!(file("ci.yml").contains("\n  pull_request:") && file("packaging-lint.yml").contains("\n  pull_request:"));
        let guard = file("own-runners-guard.yml");
        assert!(guard.contains("types: [completed]"), "one guard run per other run");
        assert!(guard.contains("paths: [\".github/workflows/**\"]"), "the file check only when files change");
    }

    #[test]
    fn a_pr_whose_checks_all_pass_is_merged() {
        let wf = file("auto-merge.yml");
        assert!(wf.contains("types: [opened, reopened, synchronize, ready_for_review]"));
        assert!(wf.contains("workflows: [\"CI\", \"Packaging lint\", \"Fork automation\"]"), "and again whenever a check finishes");
        assert!(include_str!("../../packaging/fork/lib.sh").contains("&filter=latest"), "only each check's latest attempt counts");
        assert!(wf.contains("run: packaging/fork/auto-merge.sh \"$PR\"") && wf.contains("persist-credentials: false"));
        assert!(
            wf.contains("!github.event.pull_request.draft") && wf.contains("head.repo.full_name == github.repository"),
            "never drafts or other repositories"
        );
        let script = include_str!("../../packaging/fork/auto-merge.sh");
        assert!(script.contains("merge_when_green") && script.contains("MERGE_IGNORE_CHECK:-merge when green"), "waits for every check but its own");
        assert!(file("own-runners-guard.yml").contains("\"Auto-merge\""));
    }

    #[test]
    fn the_sync_keeps_every_one_of_the_forks_workflows() {
        // sync-upstream.sh deletes any workflow not in FORK_WORKFLOWS: a workflow left off it is
        // deleted by the next upstream merge (fork-automation.yml and auto-merge.yml were).
        let script = include_str!("../../packaging/fork/sync-upstream.sh");
        let line = script.lines().find(|l| l.starts_with("FORK_WORKFLOWS=")).expect("FORK_WORKFLOWS in sync-upstream.sh");
        let listed: Vec<&str> = line.trim_start_matches("FORK_WORKFLOWS=").trim_matches('"').split_whitespace().collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows");
        let mut files: Vec<String> =
            std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".yml")).collect();
        files.sort();
        for f in &files {
            assert!(listed.contains(&f.as_str()), "{f} is not in FORK_WORKFLOWS: the next upstream sync deletes it");
        }
        for (name, _) in ALL {
            assert!(files.iter().any(|f| f == name), "{name} is gated here but not in .github/workflows");
        }
    }

    #[test]
    fn no_workflow_pins_work_to_one_core() {
        let serial = [
            "-j 1",
            "-j1",
            "--jobs 1",
            "--jobs=1",
            "--test-threads=1",
            "--test-threads 1",
            "CARGO_BUILD_JOBS: 1",
            "CARGO_BUILD_JOBS=1",
            "RUST_TEST_THREADS: 1",
            "RUST_TEST_THREADS=1",
        ];
        for (file, text) in ALL {
            for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
                for s in serial {
                    let pinned = line.split(s).skip(1).any(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()));
                    assert!(!pinned, "{file}: `{}` runs on one core", line.trim());
                }
            }
        }
    }

    #[test]
    fn the_job_splitter_sees_every_ci_job() {
        let names: Vec<String> = jobs(CI).into_iter().map(|(n, _)| n).collect();
        for expected in ["test", "deny", "fuzz"] {
            assert!(names.iter().any(|n| n == expected), "{expected} in {names:?}");
        }
    }
}
