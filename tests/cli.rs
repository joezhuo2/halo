use std::process::{Command, Output};

fn halo(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_halo-ide")).args(args).output().expect("run halo")
}

#[test]
fn version_prints_and_exits() {
    for flag in ["--version", "-v"] {
        let out = halo(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("halo {}\n", env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn help_prints_and_exits() {
    for flag in ["--help", "-h"] {
        let out = halo(&["some-file.rs", flag]);
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Usage: halo [options] [paths...]") && text.contains("--wait"), "{text}");
    }
}

#[test]
fn unknown_option_fails_with_help() {
    let out = halo(&["--frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("unknown option '--frobnicate'") && text.contains("Usage: halo"), "{text}");
}
