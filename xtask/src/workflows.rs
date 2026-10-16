//! Checks on the GitHub Actions workflows that only tests need: CI and CD use the host's cores in
//! parallel (the owner's rule, CLAUDE.md), so none of these may quietly fall back to serial.

#[cfg(test)]
mod tests {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const ALL: [(&str, &str); 4] = [
        ("ci.yml", CI),
        ("release.yml", include_str!("../../.github/workflows/release.yml")),
        ("packaging-lint.yml", include_str!("../../.github/workflows/packaging-lint.yml")),
        ("own-runners-guard.yml", include_str!("../../.github/workflows/own-runners-guard.yml")),
    ];

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
