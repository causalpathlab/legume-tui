//! The script beside a run, and the command laid out one flag to a line.

use super::*;

fn words(ws: &[&str]) -> Vec<String> {
    ws.iter().map(|w| w.to_string()).collect()
}

#[test]
fn each_input_and_each_flag_with_its_values_has_a_line() {
    let lines = command_lines(
        "tool",
        &words(&[
            "diff",
            "/d/a.zarr",
            "/d/b zarr",
            "-x",
            "1",
            "2",
            "--min",
            "-1",
            "-o",
            ".",
        ]),
        1,
    );
    assert_eq!(
        lines,
        [
            "tool diff",
            "/d/a.zarr",
            "'/d/b zarr'",
            "-x 1 2",
            "--min -1",
            "-o ."
        ]
    );
}

#[test]
fn words_are_quoted_only_when_the_shell_would_split_them() {
    assert_eq!(quote("a/b.gz"), "a/b.gz");
    assert_eq!(quote(""), "''");
    assert_eq!(quote("it's"), r"'it'\''s'");
    assert_eq!(line(&words(&["a", "b c"])), "a 'b c'");
    assert_eq!(program_word("data-beans"), "\"${DATA_BEANS:-data-beans}\"");
}

#[test]
fn the_script_runs_from_its_folder_refuses_a_finished_run_and_is_never_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let script = Script {
        program: "cocoa",
        version: "1.0",
        made_by: "cocoa diff",
        name: "out.cmd.sh",
        guard: Some("out.effect.parquet"),
        cd: true,
        env: &[("RUST_LOG", "info")],
        head: 1,
    };
    let argv = words(&["diff", "x.zarr", "-o", "out"]);
    let text = text(&script, &argv);
    assert!(text.starts_with("#!/usr/bin/env bash\n# Made by `cocoa diff` (cocoa 1.0)"));
    assert!(text.contains("cd \"$(dirname \"$0\")\"\n"));
    assert!(text.contains("export RUST_LOG=\"${RUST_LOG:-info}\"\n"));
    assert!(text.contains("if [ -e out.effect.parquet ]; then"));
    assert!(
        text.ends_with("\"${COCOA:-cocoa}\" diff \\\n  x.zarr \\\n  -o out\n"),
        "{text}"
    );

    let path = tmp.path().join("out.cmd.sh");
    write(&path, &script, &argv).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
    let again = write(&path, &script, &argv).unwrap_err();
    assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(again.to_string().contains("out.cmd.sh"), "{again}");
}
