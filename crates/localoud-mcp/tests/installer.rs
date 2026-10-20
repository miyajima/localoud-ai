#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

#[test]
fn installer_builds_and_seeds_config_from_fixture_repo_root() -> std::io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let repo = fixture.path();
    let plugin = repo.join("plugins/localoud-mcp");
    let scripts = plugin.join("scripts");
    let fake_bin = fixture.path().join("fake-bin");
    let home = fixture.path().join("home");
    fs::create_dir_all(&scripts)?;
    fs::create_dir_all(&fake_bin)?;

    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
    fs::copy(
        project_root.join("plugins/localoud-mcp/scripts/install.sh"),
        scripts.join("install.sh"),
    )?;
    fs::copy(
        project_root.join("plugins/localoud-mcp/mcp-config.example.json"),
        plugin.join("mcp-config.example.json"),
    )?;

    let fake_cargo = fake_bin.join("cargo");
    fs::write(
        &fake_cargo,
        "#!/bin/sh\n\
         set -eu\n\
         test \"$1 $2 $3 $4\" = \"build --release -p localoud-mcp\"\n\
         printf '%s' \"$PWD\" > \"$FAKE_CARGO_CWD\"\n\
         mkdir -p \"$PWD/target/release\"\n\
         printf '%s\\n' '#!/bin/sh' 'echo fixture-binary' > \"$PWD/target/release/localoud-mcp\"\n\
         chmod +x \"$PWD/target/release/localoud-mcp\"\n",
    )?;
    fs::set_permissions(&fake_cargo, fs::Permissions::from_mode(0o755))?;

    let cargo_cwd = fixture.path().join("cargo-cwd");
    let original_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{original_path}", fake_bin.display());
    let output = Command::new("/bin/sh")
        .arg(scripts.join("install.sh"))
        .current_dir(repo)
        .env("HOME", &home)
        .env("PATH", path)
        .env("FAKE_CARGO_CWD", &cargo_cwd)
        .output()?;
    assert!(
        output.status.success(),
        "installer failed (status {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let expected_binary = repo.join("target/release/localoud-mcp");
    assert_eq!(fs::read_to_string(cargo_cwd)?, repo.to_string_lossy());
    assert!(
        expected_binary.is_file(),
        "fake cargo must build in the repo root"
    );
    let installed = home.join(".local/bin/localoud-mcp");
    assert!(fs::symlink_metadata(&installed)?.file_type().is_symlink());
    assert_eq!(
        fs::canonicalize(installed)?,
        fs::canonicalize(expected_binary)?
    );

    let seeded_config = fs::read_to_string(home.join(".config/localoud/mcp-projects.json"))?;
    let source_config = fs::read_to_string(plugin.join("mcp-config.example.json"))?;
    assert_eq!(seeded_config, source_config);
    Ok(())
}
