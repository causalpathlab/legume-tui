//! A command's flags as a form: rows from clap, the command line they make,
//! clap's check of it and the flag its complaint blames.

use super::*;
use clap::{CommandFactory, Parser, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Zarr,
    H5,
}

#[derive(Parser, Debug)]
#[command(name = "tool")]
struct Args {
    #[arg(required = true)]
    data: Vec<String>,
    #[arg(short, long, required = true)]
    output: String,
    #[arg(long, default_value_t = 10, help = "Projection dimension")]
    proj_dim: usize,
    #[arg(long, value_delimiter = ',', help = "Topic files")]
    topics: Option<Vec<String>>,
    #[arg(long, value_enum, default_value = "zarr")]
    backend: Backend,
    #[arg(long, value_enum)]
    scale: Option<Backend>,
    #[arg(long, default_value_t = false)]
    preload: bool,
    #[arg(long = "no-zip", default_value_t = true, action = clap::ArgAction::SetFalse)]
    zip: bool,
    #[arg(long, help_heading = "Common", default_value_t = 1.0)]
    a0: f32,
    #[arg(long, hide = true, default_value_t = 3)]
    secret: usize,
}

fn cmd() -> Command {
    let mut c = Args::command();
    c.build();
    c
}

fn form() -> Form {
    Form::new(&cmd(), |a| a.get_long() != Some("output"))
}

#[test]
fn every_long_flag_is_a_row_of_its_kind() {
    let f = form();
    let longs: Vec<&str> = f.fields.iter().map(|f| f.long.as_str()).collect();
    // `Common` first; the positional, the owned flag and help are no rows.
    assert_eq!(longs[0], "a0");
    for l in [
        "proj-dim", "topics", "backend", "scale", "preload", "no-zip", "secret",
    ] {
        assert!(longs.contains(&l), "{l}");
    }
    assert!(!longs.contains(&"output") && !longs.contains(&"help"));
    assert_eq!(f.get("preload").unwrap().kind, Kind::Flag { on: true });
    assert_eq!(f.get("no-zip").unwrap().kind, Kind::Flag { on: false });
    assert_eq!(
        f.get("backend").unwrap().kind,
        Kind::Choice(vec!["zarr".into(), "h5".into()])
    );
    // A choice with no default can be unset.
    assert_eq!(
        f.get("scale").unwrap().kind,
        Kind::Choice(vec![String::new(), "zarr".into(), "h5".into()])
    );
    assert!(f.get("secret").unwrap().advanced);
    assert_eq!(f.get("topics").unwrap().many, Many::Joined(','));
    assert!(!f.get("proj-dim").unwrap().required);
}

#[test]
fn only_changed_rows_reach_the_command_line_and_clap_takes_them() {
    let mut f = form();
    assert!(f.argv().is_empty());
    f.get_mut("proj-dim").unwrap().set_text("12");
    f.get_mut("topics").unwrap().set_text("a.gz b.gz");
    f.get_mut("no-zip").unwrap().toggle();
    f.get_mut("backend").unwrap().cycle(1);
    f.get_mut("a0").unwrap().set_text("-0.5");
    assert_eq!(f.changed(), 5);
    let argv = f.argv();
    assert_eq!(
        argv,
        [
            "--a0=-0.5",
            "--proj-dim",
            "12",
            "--topics",
            "a.gz,b.gz",
            "--backend",
            "h5",
            "--no-zip"
        ]
    );
    let mut line: Vec<String> = ["tool", "x.zarr", "-o", "out"].map(String::from).to_vec();
    line.extend(argv);
    let m = parse(&cmd(), &line).unwrap();
    assert_eq!(m.get_one::<usize>("proj_dim"), Some(&12));
    assert!(!m.get_flag("zip"));
    // Empty text is the default again.
    f.get_mut("proj-dim").unwrap().set_text("  ");
    assert!(f.get("proj-dim").unwrap().is_default());
}

#[test]
fn a_rejected_value_is_blamed_on_its_row() {
    let mut f = form();
    f.get_mut("proj-dim").unwrap().set_text("many");
    let mut line: Vec<String> = ["tool", "x.zarr", "-o", "out"].map(String::from).to_vec();
    line.extend(f.argv());
    let complaint = check(&cmd(), &line).unwrap_err();
    assert!(!complaint.starts_with("error:"), "{complaint}");
    assert_eq!(blamed(&complaint, &f.fields), Some("proj-dim"));
    // A missing positional names no row.
    let complaint = check(&cmd(), &["tool".into(), "-o".into(), "out".into()]).unwrap_err();
    assert!(complaint.contains("required"), "{complaint}");
    assert_eq!(blamed(&complaint, &f.fields), None);
}

#[test]
fn prefill_takes_what_the_command_line_gave() {
    let mut f = form();
    let m = cmd()
        .try_get_matches_from(["tool", "x", "-o", "o", "--proj-dim", "3", "--no-zip"])
        .unwrap();
    f.prefill(&m);
    assert_eq!(f.get("proj-dim").unwrap().value, "3");
    assert!(f.argv().contains(&"--no-zip".to_string()));
    assert_eq!(f.changed(), 2);
}

#[test]
fn a_changed_row_shows_its_default_and_a_complaint() {
    let look = Look::terminal();
    let mut f = form();
    let field = f.get_mut("proj-dim").unwrap();
    field.set_text("4");
    let line = row_line(
        field,
        true,
        10,
        &look,
        look.page,
        field.shown(),
        Some("bad"),
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.starts_with("▸ * proj-dim  4"), "{text}");
    assert!(
        text.contains("(default 10)") && text.ends_with("bad"),
        "{text}"
    );
    let scale = f.get("scale").unwrap();
    assert_eq!(scale.shown(), "(unset)");
    assert_eq!(f.get("preload").unwrap().shown(), "off");
}

#[test]
fn only_keeps_the_named_flags_in_their_order() {
    let f = Form::only(&cmd(), &["backend", "proj-dim"]);
    let longs: Vec<&str> = f.fields.iter().map(|f| f.long.as_str()).collect();
    assert_eq!(longs, ["backend", "proj-dim"]);
}

#[test]
fn several_values_are_laid_out_as_the_flag_takes_them() {
    let vs = || vec!["a".to_string(), "-b".to_string()];
    assert_eq!(emit("--f", Many::One, vs()), ["--f", "a"]);
    assert_eq!(emit("--f", Many::Joined(','), vs()), ["--f", "a,-b"]);
    assert_eq!(emit("--f", Many::Listed, vs()), ["--f", "a", "-b"]);
    assert_eq!(emit("--f", Many::Repeated, vs()), ["--f", "a", "--f=-b"]);
    assert!(emit("--f", Many::One, vec![]).is_empty());
}
