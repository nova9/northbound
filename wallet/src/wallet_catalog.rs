use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WalletFile {
    pub name: String,
    pub path: PathBuf,
}

pub(crate) fn read_wallet_files(directory: &Path) -> Result<Vec<WalletFile>, String> {
    let mut wallets = Vec::new();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(wallets),
        Err(error) => return Err(format!("Could not read the wallet folder: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not read a wallet entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Could not inspect a wallet entry: {error}"))?;
        let path = entry.path();
        if !file_type.is_file() || path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "A wallet filename is not valid text.".to_owned())?;
        wallets.push(WalletFile {
            name: name.to_owned(),
            path,
        });
    }
    wallets.sort_by_key(|wallet| wallet.name.to_lowercase());
    Ok(wallets)
}

pub(crate) fn find_new_wallet_path(directory: &Path, name: &str) -> Result<WalletFile, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 40 {
        return Err("Enter a wallet name between 1 and 40 characters.".to_owned());
    }
    for character in name.chars() {
        if !character.is_ascii_alphanumeric() && !matches!(character, ' ' | '-' | '_') {
            return Err(
                "Use letters, numbers, spaces, hyphens, or underscores in the wallet name."
                    .to_owned(),
            );
        }
    }
    let uppercase = name.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (1..=9).any(|number| {
            uppercase == format!("COM{number}") || uppercase == format!("LPT{number}")
        });
    if reserved {
        return Err("Choose another wallet name; this name is reserved.".to_owned());
    }
    for wallet in read_wallet_files(directory)? {
        if wallet.name.eq_ignore_ascii_case(name) {
            return Err("A wallet with this name already exists. Choose another name.".to_owned());
        }
    }
    Ok(WalletFile {
        name: name.to_owned(),
        path: directory.join(format!("{name}.json")),
    })
}

pub(crate) fn remove_wallet_file(
    directory: &Path,
    wallet: &WalletFile,
    confirmed_name: &str,
    backup_confirmed: bool,
) -> Result<(), String> {
    if !backup_confirmed || confirmed_name != wallet.name {
        return Err(
            "Confirm your backup and enter the wallet name exactly before removing it.".to_owned(),
        );
    }
    // Only a regular wallet file from this catalog may be removed, never an arbitrary path.
    if !read_wallet_files(directory)?.contains(wallet) {
        return Err(
            "This wallet is no longer in the wallet folder. Reload the wallet list.".to_owned(),
        );
    }
    fs::remove_file(&wallet.path)
        .map_err(|error| format!("Could not remove the local wallet file: {error}"))
}

pub(crate) fn open_wallet_folder(folder: &Path) -> Result<(), String> {
    let folder = folder
        .canonicalize()
        .map_err(|error| format!("Could not find the wallet folder: {error}"))?;
    let status = file_explorer_command(&folder)
        .status()
        .map_err(|error| format!("Could not open the file explorer: {error}"))?;
    if !status.success() {
        return Err("The file explorer could not open the wallet folder.".to_owned());
    }
    Ok(())
}

fn file_explorer_command(folder: &Path) -> Command {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.args(["-a", "Finder"]);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    command.arg(folder);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_named_wallet_files_and_ignores_other_entries() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("Spending.json"), "encrypted wallet").unwrap();
        fs::write(
            directory.path().join("Savings.json"),
            "another encrypted wallet",
        )
        .unwrap();
        fs::write(directory.path().join("notes.txt"), "not a wallet").unwrap();
        fs::create_dir(directory.path().join("folder.json")).unwrap();
        let wallets = read_wallet_files(directory.path()).unwrap();
        assert_eq!(wallets.len(), 2);
        assert_eq!(wallets[0].name, "Savings");
        assert_eq!(wallets[1].name, "Spending");
        assert!(wallets[0].path.exists());
    }

    #[test]
    fn rejects_duplicate_unsafe_and_reserved_names() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("Savings.json"), "encrypted").unwrap();
        for name in [
            "savings",
            "",
            "../outside",
            "/outside",
            "a\\b",
            "CON",
            "LPT1",
        ] {
            assert!(
                find_new_wallet_path(directory.path(), name).is_err(),
                "{name}"
            );
        }
        let wallet = find_new_wallet_path(directory.path(), " Spending ").unwrap();
        assert_eq!(wallet.path, directory.path().join("Spending.json"));
    }

    #[test]
    fn removal_requires_confirmation_and_preserves_other_wallets() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("Savings.json"), "encrypted").unwrap();
        fs::write(
            directory.path().join("Spending.json"),
            "other encrypted wallet",
        )
        .unwrap();
        let wallet = read_wallet_files(directory.path()).unwrap().remove(0);
        assert!(remove_wallet_file(directory.path(), &wallet, "Savings", false).is_err());
        assert!(remove_wallet_file(directory.path(), &wallet, "Wrong wallet", true).is_err());
        assert!(wallet.path.exists());
        remove_wallet_file(directory.path(), &wallet, "Savings", true).unwrap();
        assert!(!wallet.path.exists());
        assert!(directory.path().join("Spending.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_list_or_remove_symlinked_wallets() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let path = directory.path().join("Linked.json");
        std::os::unix::fs::symlink(outside.path(), &path).unwrap();
        assert!(read_wallet_files(directory.path()).unwrap().is_empty());
        let wallet = WalletFile {
            name: "Linked".into(),
            path,
        };
        assert!(remove_wallet_file(directory.path(), &wallet, "Linked", true).is_err());
        assert!(outside.path().exists());
    }

    #[test]
    fn explorer_receives_the_folder_as_one_argument() {
        let folder = Path::new("/some folder/wallets");
        let command = file_explorer_command(folder);
        assert_eq!(command.get_args().last(), Some(folder.as_os_str()));
        #[cfg(target_os = "macos")]
        assert_eq!(command.get_args().next(), Some(std::ffi::OsStr::new("-a")));
    }
}
