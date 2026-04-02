mod layout;
mod resolve;
mod scan;
mod tenants;

use std::path::Path;

pub use scan::ScanFormat;

pub fn init_project(root: &Path) -> anyhow::Result<()> {
    layout::ensure_layout(root)
}

pub fn scan_project(root: &Path, format: ScanFormat) -> anyhow::Result<()> {
    let report = scan::scan(root)?;
    scan::render_report(&report, format)
}

pub fn sync_project(root: &Path) -> anyhow::Result<()> {
    resolve::resolve(root)
}

pub fn add_tenant(root: &Path, tenant: &str) -> anyhow::Result<()> {
    tenants::add_tenant(root, tenant)
}

pub fn remove_tenant(root: &Path, tenant: &str) -> anyhow::Result<()> {
    tenants::remove_tenant(root, tenant)
}

pub fn list_tenants(root: &Path) -> anyhow::Result<Vec<String>> {
    tenants::list_tenants(root)
}

pub fn add_team(root: &Path, tenant: &str, team: &str) -> anyhow::Result<()> {
    tenants::add_team(root, tenant, team)
}

pub fn remove_team(root: &Path, tenant: &str, team: &str) -> anyhow::Result<()> {
    tenants::remove_team(root, tenant, team)
}

pub fn list_teams(root: &Path, tenant: &str) -> anyhow::Result<Vec<String>> {
    tenants::list_teams(root, tenant)
}

fn ensure_dir(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

fn write_if_missing(path: &Path, contents: &str) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_dir_and_write_if_missing_are_idempotent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let nested = dir.path().join("state/runtime");
        ensure_dir(&nested)?;
        assert!(nested.exists());

        let file = dir.path().join("config/demo.txt");
        write_if_missing(&file, "first")?;
        write_if_missing(&file, "second")?;
        assert_eq!(std::fs::read_to_string(file)?, "first");
        Ok(())
    }
}
