use super::*;

/// An interactive terminal with nothing set: the default case.
fn interactive() -> Env {
    Env {
        stdin_tty: true,
        stdout_tty: true,
        ..Env::default()
    }
}

#[test]
fn interactive_terminal_is_hosted_by_default() {
    assert!(hosts_by_default());
    assert_eq!(mode(&Options::default(), &interactive()), Mode::Launch);
}

#[test]
fn every_bypass_runs_plain() {
    let opts = Options::default();
    let flag = Options {
        no_detach: true,
        ..Options::default()
    };
    assert_eq!(mode(&flag, &interactive()), Mode::Plain(Bypass::Flag));

    let cases = [
        (
            Env {
                no_detach: true,
                ..interactive()
            },
            Bypass::Env,
        ),
        (
            Env {
                marker: true,
                ..interactive()
            },
            Bypass::Nested,
        ),
        (
            Env {
                stdin_tty: false,
                ..interactive()
            },
            Bypass::NotTerminal,
        ),
        (
            Env {
                stdout_tty: false,
                ..interactive()
            },
            Bypass::NotTerminal,
        ),
        (
            Env {
                multiplexer: true,
                ..interactive()
            },
            Bypass::Multiplexer,
        ),
    ];
    for (env, why) in cases {
        assert_eq!(mode(&opts, &env), Mode::Plain(why), "{env:?}");
    }
}

#[test]
fn the_flag_wins_over_asking_to_detach() {
    let both = Options {
        no_detach: true,
        detach: true,
        ..Options::default()
    };
    assert_eq!(mode(&both, &interactive()), Mode::Plain(Bypass::Flag));
}

#[test]
fn the_hosted_program_is_hosted_and_a_nested_launch_is_not() {
    // The program a host started: the marker and the channel its parent
    // handed it.
    let program = Env {
        marker: true,
        control: true,
        ..interactive()
    };
    assert_eq!(mode(&Options::default(), &program), Mode::Hosted);
    // Even with no terminal check passing: its terminal is the host's pty.
    let program = Env {
        stdin_tty: false,
        ..program
    };
    assert_eq!(mode(&Options::default(), &program), Mode::Hosted);
    // Something it started sees the marker, not the channel: plain.
    let nested = Env {
        marker: true,
        control: false,
        ..interactive()
    };
    assert_eq!(
        mode(&Options::default(), &nested),
        Mode::Plain(Bypass::Nested)
    );
    // A channel without the marker is not ours.
    let stray = Env {
        control: true,
        ..interactive()
    };
    assert_eq!(mode(&Options::default(), &stray), Mode::Launch);
}

#[test]
fn environment_is_read_from_the_process() {
    // Whatever this test runs under, the marker and channel are not set,
    // so the read cannot claim to be hosted.
    let env = Env::read();
    if std::env::var_os(crate::SESSION_VAR).is_none() {
        assert!(!env.marker);
        assert!(!env.control);
    }
}
