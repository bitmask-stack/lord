use std::{fs, path::PathBuf, process::Command};

fn workspace_cargo_toml_files() -> Vec<PathBuf> {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  let mut files = vec![root.join("Cargo.toml")];
  for entry in fs::read_dir(root.join("crates")).expect("crates dir") {
    files.push(entry.expect("crate entry").path().join("Cargo.toml"));
  }
  files
}

#[test]
fn no_redb_dependency_in_cargo_toml() {
  for cargo_toml in workspace_cargo_toml_files() {
    let cargo = fs::read_to_string(&cargo_toml).expect("Cargo.toml");
    assert!(
      !cargo.lines().any(|line| line.trim().starts_with("redb")),
      "redb must not be listed in {}",
      cargo_toml.display()
    );
  }
}

#[test]
fn no_redb_crate_imports_in_src() {
  let output = Command::new("rg")
    .args([
      "-e",
      "redb::",
      "-e",
      "use redb",
      "-g",
      "!CHANGELOG.md",
      "src/",
      "crates/",
    ])
    .output()
    .expect("rg");

  assert!(
    output.status.success() || output.status.code() == Some(1),
    "rg failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert!(
    output.stdout.is_empty(),
    "unexpected redb crate imports:\n{}",
    String::from_utf8_lossy(&output.stdout)
  );
}

#[test]
fn forbid_script_passes() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  let output = Command::new(root.join("bin/forbid"))
    .current_dir(&root)
    .output()
    .expect("bin/forbid");

  assert!(
    output.status.success(),
    "bin/forbid failed:\nstdout: {}\nstderr: {}",
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr)
  );
}
