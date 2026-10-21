use std::process::{Command, Output};

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_splunk"));
    command.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        command.env("PATH", path);
    }
    command.env("HOME", home).args(args).output().unwrap()
}

#[test]
fn bundled_skill_commands_work_without_splunk_credentials() {
    let home = tempfile::tempdir().unwrap();
    let list = run(home.path(), &["skills", "list", "--json"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let skills: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(
        skills
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "splunk-cli" && item["bundled"] == true)
    );

    let show = run(home.path(), &["skills", "show", "splunk-cli"]);
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("splunk search-async"));
}

#[test]
fn bundled_install_selects_host_and_preserves_conflicts() {
    let home = tempfile::tempdir().unwrap();
    let first = run(home.path(), &["skills", "install", "--target", "agents"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let skill = home.path().join(".agents/skills/splunk-cli/SKILL.md");
    assert!(skill.is_file());
    let before = std::fs::read(&skill).unwrap();
    let conflict = run(home.path(), &["skills", "install", "--target", "agents"]);
    assert_eq!(conflict.status.code(), Some(1));
    assert_eq!(std::fs::read(&skill).unwrap(), before);
    let forced = run(
        home.path(),
        &["skills", "install", "--target", "agents", "--force"],
    );
    assert!(forced.status.success());
}

#[test]
fn list_discovers_skills_from_an_explicit_root() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("additional/group/example");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("SKILL.md"),
        "---\nname: local-example\ndescription: Local skill\n---\nInstructions\n",
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_splunk"));
    command.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        command.env("PATH", path);
    }
    let output = command
        .args(["skills", "list", "--json"])
        .env("HOME", home.path())
        .env("SPLUNK_CLI_SKILLS_PATH", home.path().join("additional"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let skills: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        skills
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["name"] == "local-example")
    );
}
