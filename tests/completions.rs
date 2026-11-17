use std::process::Command;

#[test]
fn lrat_completions_prints_a_script_for_each_shell() {
    for (shell, marker) in [
        ("bash", "complete -F _lrat"),
        ("zsh", "#compdef lrat"),
        ("fish", "complete -c lrat"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lrat"))
            .args(["completions", shell])
            .output()
            .unwrap();

        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains(marker), "{shell}:\n{script}");
        assert!(script.contains("auth"), "{shell} omits the subcommands");
    }
}
