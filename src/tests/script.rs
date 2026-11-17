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
    let script = Script::new("cocoa", "1.0", "cocoa diff", "out.cmd.sh")
        .guard("out.effect.parquet")
        .cd(true)
        .env(&[("RUST_LOG", "info")])
        .head(1);
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

/// senna's `run::tui::script::text` for `senna run` with out `runs/r`,
/// as it wrote it before it moved here.
const SENNA: &str = r#"#!/usr/bin/env bash
# Made by `senna run` (senna 0.9.1). Run it again with: bash runs/r.cmd.sh
set -euo pipefail
cd "$(dirname "$0")"/..
export RUST_LOG="${RUST_LOG:-info}"
out=runs/r
if [ -f "${out}.senna.json" ]; then
  echo "${out}.senna.json exists; move it away to run again" >&2
  exit 1
fi
"${SENNA:-senna}" topic \
  data/a.zarr \
  'data/b c.zarr' \
  --out "$out" \
  --k 10 \
  --batch x y
"#;

#[test]
fn senna_s_script_is_made_with_out_as_a_variable_run_from_where_out_is_relative_to() {
    let out = "runs/r";
    let name = format!("{out}.cmd.sh");
    let script = Script::new("senna", "0.9.1", "senna run", &name)
        .up(depth(out))
        .env(&[("RUST_LOG", "info")])
        .head(1)
        .out(out)
        .result("senna.json");
    let argv = words(&[
        "topic",
        "data/a.zarr",
        "data/b c.zarr",
        "--out",
        out,
        "--k",
        "10",
        "--batch",
        "x",
        "y",
    ]);
    assert_eq!(text(&script, &argv), SENNA);
    assert_eq!(
        script.command_lines(&argv)[3],
        "--out \"$out\"",
        "the preview names the prefix as the script does"
    );
    assert_eq!(depth("r"), 0);
    assert_eq!(depth("/abs/r"), 0);
    assert_eq!(depth("a/b/r"), 2);
}

#[cfg(unix)]
#[test]
fn a_script_written_over_replaces_the_one_there_and_runs() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("fit.cmd.sh");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let script = Script::new("true", "1", "faba fit", "fit.cmd.sh").cd(true);
    write_over(&path, &script, &words(&["--x", "1"])).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("\"${TRUE:-true}\" \\\n  --x 1\n"), "{text}");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
    let ran = std::process::Command::new("bash")
        .arg(&path)
        .status()
        .unwrap();
    assert!(ran.success());
}
