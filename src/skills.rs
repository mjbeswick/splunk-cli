use clap::ValueEnum;
use serde::Serialize;
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

pub const BUNDLED: &str = include_str!("../skills/splunk-cli/SKILL.md");
const NAME: &str = "splunk-cli";

#[derive(Clone, Debug, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: Option<PathBuf>,
    pub bundled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Target {
    Claude,
    Opencode,
    Agents,
}
impl Target {
    fn relative_dir(self) -> &'static str {
        match self {
            Self::Claude => ".claude/skills",
            Self::Opencode => ".config/opencode/skills",
            Self::Agents => ".agents/skills",
        }
    }
}

fn parse(text: &str, path: Option<PathBuf>, bundled: bool) -> Option<Skill> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        let line = line.trim();
        if line == "---" {
            closed = true;
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(['"', '\'']);
            match key.trim() {
                "name" if !value.is_empty() => name = Some(value.to_owned()),
                "description" if !value.is_empty() => description = Some(value.to_owned()),
                _ => {}
            }
        }
    }
    if !closed {
        return None;
    }
    Some(Skill {
        name: name?,
        description: description?,
        path,
        bundled,
    })
}

pub fn discover() -> Vec<Skill> {
    let mut out = vec![parse(BUNDLED, None, true).expect("bundled skill has frontmatter")];
    let mut roots = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        roots.extend([
            cwd.join(".agents/skills"),
            cwd.join(".claude/skills"),
            cwd.join(".opencode/skills"),
            cwd.join("skills"),
        ]);
    }
    if let Some(home) = home_dir() {
        roots.extend([
            home.join(".agents/skills"),
            home.join(".claude/skills"),
            home.join(".config/opencode/skills"),
        ]);
    }
    if let Some(extra) = env::var_os("SPLUNK_CLI_SKILLS_PATH") {
        roots.extend(env::split_paths(&extra));
    }
    let mut visited = HashSet::new();
    for root in roots {
        discover_under(&root, 0, &mut visited, &mut out);
    }
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| b.bundled.cmp(&a.bundled))
    });
    out
}
fn discover_under(root: &Path, depth: usize, visited: &mut HashSet<PathBuf>, out: &mut Vec<Skill>) {
    if depth > 8 {
        return;
    }
    let Ok(root) = fs::canonicalize(root) else {
        return;
    };
    if !visited.insert(root.clone()) {
        return;
    }
    let file = root.join("SKILL.md");
    if let Ok(meta) = fs::symlink_metadata(&file) {
        if meta.file_type().is_file() && meta.len() <= 256 * 1024 {
            if let Ok(text) = fs::read_to_string(&file) {
                if let Some(skill) = parse(&text, Some(file), false) {
                    out.push(skill);
                }
            }
        }
        return;
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                discover_under(&entry.path(), depth + 1, visited, out);
            }
        }
    }
}
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
}

pub fn show(name: &str) -> Result<String, String> {
    let matches: Vec<_> = discover().into_iter().filter(|s| s.name == name).collect();
    match matches.as_slice() {
        [s] if s.bundled => Ok(BUNDLED.to_owned()),
        [s] => fs::read_to_string(s.path.as_ref().expect("path")).map_err(|e| e.to_string()),
        [_, ..] if matches.iter().any(|s| s.bundled) => Ok(BUNDLED.to_owned()),
        [] => Err(format!(
            "skill `{name}` not found; run `splunk skills list`"
        )),
        _ => Err(format!(
            "skill `{name}` is ambiguous; inspect `splunk skills list`"
        )),
    }
}

pub fn install(targets: &[Target], force: bool) -> Result<Vec<PathBuf>, String> {
    let defaults = [Target::Claude, Target::Opencode, Target::Agents];
    let targets = if targets.is_empty() {
        &defaults[..]
    } else {
        targets
    };
    let home = home_dir().ok_or_else(|| {
        "Cannot install agent skills because the home directory is unknown".to_owned()
    })?;
    let mut paths = Vec::new();
    for target in targets {
        let dir = home.join(target.relative_dir()).join(NAME);
        let path = dir.join("SKILL.md");
        match fs::symlink_metadata(&path) {
            Ok(_) if !force => {
                return Err(format!(
                    "{} already exists; use --force to replace it",
                    path.display()
                ));
            }
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(format!(
                    "refusing to replace non-regular path {}",
                    path.display()
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
            _ => {}
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        if force && path.exists() {
            let temp = dir.join(format!(".SKILL.md.{}.tmp", std::process::id()));
            fs::write(&temp, BUNDLED).map_err(|e| e.to_string())?;
            fs::rename(temp, &path).map_err(|e| e.to_string())?;
        } else {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| e.to_string())?;
            use std::io::Write;
            f.write_all(BUNDLED.as_bytes()).map_err(|e| e.to_string())?;
        }
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frontmatter_parse() {
        assert_eq!(
            parse("---\nname: x\ndescription: 'hello'\n---\n", None, false)
                .unwrap()
                .description,
            "hello"
        );
        assert!(parse("---\nname: x", None, false).is_none());
    }
    #[test]
    fn install_conflict_and_force() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".agents/skills/splunk-cli/SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "local").unwrap();
        let old = env::var_os("HOME");
        unsafe {
            env::set_var("HOME", home.path());
        }
        assert!(install(&[Target::Agents], false).is_err());
        install(&[Target::Agents], true).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), BUNDLED);
        unsafe {
            match old {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }
}
