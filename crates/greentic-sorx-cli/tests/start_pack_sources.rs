//! `start` reads its pack from a local path or an `oci://` reference, and
//! its `--answers` from a file or `env:NAME`. These are process-boundary
//! tests: the failures below must surface as a CLI exit + stderr message,
//! not just as a library-level `Result`.

use assert_cmd::Command;

fn sorx() -> Command {
    Command::cargo_bin("greentic-sorx").expect("binary")
}

#[test]
fn answers_from_an_unset_variable_fail_naming_the_variable() {
    let out = sorx()
        .args([
            "start",
            "does-not-matter.gtpack",
            "--answers",
            "env:SORX_TEST_UNSET_ANSWERS",
            "--dry-run",
        ])
        .env_remove("SORX_TEST_UNSET_ANSWERS")
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("SORX_TEST_UNSET_ANSWERS"), "{stderr}");
}

#[test]
fn an_unreachable_oci_reference_fails_before_serving() {
    let out = sorx()
        .args([
            "start",
            "oci://127.0.0.1:9/greentic/none:t",
            "--answers",
            "env:SORX_TEST_ANSWERS",
            "--dry-run",
        ])
        .env("SORX_TEST_ANSWERS", "{}")
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot pull pack oci://127.0.0.1:9/greentic/none:t"),
        "{stderr}"
    );
}
