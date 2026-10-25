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
        // `contains`: the suite runs inside packaging/fork/private-tmp.sh, after the wrapper's name.
        assert!(cmds.iter().any(|c| c.contains("cargo nextest run --workspace")), "the tests run under nextest");
        assert!(cmds.iter().any(|c| c.contains("cargo test --workspace --doc")), "and the doctests, which nextest skips");
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
        assert!(sync.contains("schedule:") && sync.contains("cron: \"0 6 * * *\""), "every day");
        assert!(sync.contains("cargo xtask version check-upstream"), "then proves the version is on top of upstream's release");
        let version = jobs(file("release.yml")).into_iter().find(|(n, _)| n == "version").expect("version job").1;
        assert!(version.contains("cargo xtask version check-upstream"), "and Release refuses a version that isn't");
        assert!(sync.contains("workflow_dispatch:"));
        assert!(sync.contains("run: packaging/fork/sync-upstream.sh"));
        assert!(sync.contains("persist-credentials: false"), "never pushes with the workflow's token");
        let script = include_str!("../../packaging/fork/sync-upstream.sh");
        let lib = include_str!("../../packaging/fork/lib.sh");
        assert!(script.contains("gh pr create") && script.contains("merge_when_green_or_fix \"$pr\""));
        assert!(lib.contains("gh pr merge \"$1\" --repo \"$REPO\" --merge --delete-branch") && lib.contains("merge_pr \"$pr\""));
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
    fn every_job_asks_for_the_own_runners_by_role_never_by_machine() {
        // own-ci-runners: `primary` is the one role label (the primary host first, the other after
        // the takeover delay); a host's name never appears in a public workflow (machine-privacy).
        for (file, text) in ALL {
            for line in text.lines().map(str::trim).filter(|l| l.starts_with("runs-on:")) {
                let labels = line.trim_start_matches("runs-on:").trim();
                assert!(
                    labels == "[self-hosted, linux]" || labels == "[self-hosted, linux, primary]",
                    "{file}: `{line}` asks for something other than the own runners' generic labels"
                );
            }
        }
    }

    #[test]
    fn actionlint_knows_every_role_label_a_job_asks_for() {
        // actionlint (Packaging lint) refuses a runs-on label it does not know: every label beyond
        // the generic self-hosted/linux must be listed in .github/actionlint.yaml.
        let config = include_str!("../../.github/actionlint.yaml");
        let known: Vec<&str> = config.lines().map(str::trim).filter_map(|l| l.strip_prefix("- ")).collect();
        for (file, text) in ALL {
            for line in text.lines().map(str::trim).filter(|l| l.starts_with("runs-on:")) {
                let labels = line.trim_start_matches("runs-on:").trim().trim_start_matches('[').trim_end_matches(']');
                for label in labels.split(',').map(str::trim).filter(|l| !["self-hosted", "linux"].contains(l)) {
                    assert!(known.contains(&label), "{file}: `{label}` is not in .github/actionlint.yaml, so actionlint fails");
                }
            }
        }
    }

    #[test]
    fn the_llms_fixes_are_guarded_and_read_a_log_digest() {
        // What the on-call LLM's fixes may never change is checked on its commits before any push
        // (both the sync's fix loop and the CI-fix path go through llm_fix), and the CI log it reads
        // is a digest small enough for the model's context, not the raw ~900 KB log.
        let lib = include_str!("../../packaging/fork/lib.sh");
        let fix = lib.split("llm_fix() {").nth(1).and_then(|r| r.split("\n}\n").next()).expect("llm_fix in lib.sh");
        assert!(fix.contains("llm_guard \"$before\""), "llm_fix guards the LLM's commits");
        let protected: Vec<&str> = lib
            .lines()
            .find_map(|l| l.strip_prefix("LLM_PROTECTED=(")?.strip_suffix(')'))
            .expect("LLM_PROTECTED in lib.sh")
            .split_whitespace()
            .collect();
        for p in [".github/", "packaging/fork/", "xtask/src/workflows.rs", "deny.toml", ".repo-structure.toml", "CLAUDE.md", "AGENTS.md"] {
            assert!(protected.contains(&p), "{p} is protected from the LLM's fixes");
        }
        assert!(lib.contains("#\\[ignore") && lib.contains("(true|:)"), "test-disabling and failure-muting lines are refused");
        assert!(lib.contains("done | ci_log_digest"), "the failed log is digested");
        assert!(lib.contains("--autofix \"assets=bash -c '. packaging/fork/lib.sh && rehash_catalogs'\""), "an edited catalog is re-hashed");
    }

    #[test]
    fn the_sync_runs_on_the_primary_host_and_hands_what_a_merge_breaks_to_the_llm() {
        let sync = jobs(file("sync-upstream.yml")).into_iter().find(|(n, _)| n == "sync").expect("a sync job").1;
        assert!(sync.contains("runs-on: [self-hosted, linux, primary]"), "the primary host first, the other when it is down");
        assert!(file("sync-upstream.yml").contains("workflow_dispatch:"), "it can be started on demand");
        assert!(sync.contains("Saturate host threads"), "its fix loop builds and tests the workspace");
        // The owner turned the LLM off (2026-10-25): every run merges without it, and what it would
        // have handled stops for a person, with an issue that tells them.
        assert!(sync.contains("run: echo \"SYNC_NO_LLM=1\" >> \"$GITHUB_ENV\"") && !sync.contains("health"), "the sync never pages the LLM");
        let lib_text = include_str!("../../packaging/fork/lib.sh");
        assert!(lib_text.contains("gh issue create --repo \"$REPO\""), "a stop for a person opens an issue");
        let script = include_str!("../../packaging/fork/sync-upstream.sh");
        assert!(script.contains("\"${ONCALL_LLM:-oncall-llm}\" resolve --repo \"$ROOT\""), "conflicts go to the LLM");
        assert!(script.contains("llm_fix \"Sync fix\" || die"), "then CI's checks, fixed until they pass");
        assert!(script.contains("merge_when_green_or_fix \"$pr\""), "and a failing PR check goes back to it");
        let lib = include_str!("../../packaging/fork/lib.sh");
        assert!(lib.contains("--log-failed") && lib.contains("SYNC_CI_FIX_RETRIES:-2"), "with the failed log, a bounded number of times");
        // The fix loop's checks are CI's test job's: a check CI runs that the loop skips would merge a
        // tree the LLM never made pass.
        let test = jobs(CI).into_iter().find(|(n, _)| n == "test").expect("ci.yml test job").1;
        for c in [
            "cargo fmt --all -- --check",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "cargo nextest run --workspace --no-fail-fast",
            "cargo test --workspace --doc",
            "cargo xtask layers",
            "cargo xtask assets",
            "cargo check -p pdfcraft-cli --no-default-features",
        ] {
            assert!(test.contains(c), "ci.yml's test job runs `{c}`");
            assert!(lib.contains(c), "the LLM's fix loop runs `{c}` too");
        }
        // Both run the suite in a temp directory of their own, removed afterwards: upstream's tests
        // leave ~300 folders per run, and 58,000 of them filled the runner host's /tmp.
        let contained = "packaging/fork/private-tmp.sh bash -c 'cargo nextest run --workspace --no-fail-fast && cargo test --workspace --doc'";
        assert!(test.contains(contained), "ci.yml's test job runs the suite in its own temp directory");
        assert!(lib.contains(contained), "and so does the LLM's fix loop");
        // And Packaging lint's: a merged script that doesn't parse failed the 0.13.0 sync's PR while
        // the loop, running CI's test job only, reported every check passing.
        let lint = file("packaging-lint.yml");
        for c in [
            "shellcheck -x packaging/env.sh packaging/icons.sh packaging/macos/*.sh packaging/macos/dmg/*.sh packaging/fork/*.sh",
            "xmllint --noout packaging/macos/*.plist*",
            "packaging/macos/test-install.sh",
        ] {
            assert!(lint.contains(c), "packaging-lint.yml runs `{c}`");
            assert!(lib.contains(c), "the LLM's fix loop runs `{c}` too");
        }
        // Upstream's lock, taken on a conflict, never takes a package below the fork's version.
        assert!(include_str!("../../packaging/fork/sync-upstream.sh").contains("keep_newer_locked \"$RUNNER_TMP/fork.lock\""));
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
        // A run for a finished check must never cancel the PR's own run: that left every merged PR
        // with a red, cancelled "merge when green" although all its checks had passed.
        assert!(wf.contains("group: auto-merge-${{ github.event_name }}-"), "the PR's run and the finished-check runs never share a group");
        assert!(wf.contains("AUTO_MERGE_EVENT: ${{ github.event_name }}") && script.contains("AUTO_MERGE_EVENT"), "finished-check runs never wait");
        assert!(include_str!("../../packaging/fork/lib.sh").contains("--match-head-commit"), "only the commit whose checks passed is merged");
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
