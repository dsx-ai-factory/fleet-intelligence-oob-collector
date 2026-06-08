use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub address: String,
    pub username: String,
    pub password: String,
}

pub fn load_targets(path: &str) -> Result<Vec<Target>, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut targets = Vec::new();
    let mut current: Option<Target> = None;

    for (line_no, line) in reader.lines().enumerate() {
        let line_no = line_no + 1;
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            if let Some(target) = current.take() {
                validate_target(&target)?;
                targets.push(target);
            }
            let name = name.trim();
            if name.is_empty() {
                return Err(format!("empty section name at line {line_no}").into());
            }
            current = Some(Target {
                name: name.to_string(),
                address: String::new(),
                username: String::new(),
                password: String::new(),
            });
            continue;
        }

        let Some(target) = current.as_mut() else {
            return Err(format!("key outside section at line {line_no}").into());
        };
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("invalid config line {line_no}").into());
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "address" => target.address = value.trim().to_string(),
            "username" => target.username = value.trim().to_string(),
            "password" => target.password = value.trim().to_string(),
            other => return Err(format!("unknown key {other:?} at line {line_no}").into()),
        }
    }

    if let Some(target) = current.take() {
        validate_target(&target)?;
        targets.push(target);
    }
    if targets.is_empty() {
        return Err(format!("no BMC targets found in {path}").into());
    }
    Ok(targets)
}

fn validate_target(target: &Target) -> Result<(), Box<dyn Error>> {
    if target.address.is_empty() {
        return Err(format!("target {:?} address is required", target.name).into());
    }
    if target.username.is_empty() {
        return Err(format!("target {:?} username is required", target.name).into());
    }
    if target.password.is_empty() {
        return Err(format!("target {:?} password is required", target.name).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn loads_multiple_targets_and_ignores_comments() {
        let path = write_config(
            "valid",
            r#"
            # lab BMCs
            [node-a]
            address = 10.0.0.1
            username = admin
            password = secret

            ; second target
            [node-b]
            address = https://bmc.example.com
            username = root
            password = pass
            "#,
        );

        let targets = load_targets(path.to_str().expect("path")).expect("targets");

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "node-a");
        assert_eq!(targets[0].address, "10.0.0.1");
        assert_eq!(targets[1].name, "node-b");
        assert_eq!(targets[1].address, "https://bmc.example.com");
    }

    #[test]
    fn rejects_key_outside_section() {
        let path = write_config("key-outside-section", "address = 10.0.0.1\n");

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("key outside section should fail")
            .to_string();

        assert!(error.contains("key outside section"));
    }

    #[test]
    fn rejects_unknown_key() {
        let path = write_config(
            "unknown-key",
            r#"
            [node-a]
            address = 10.0.0.1
            username = admin
            password = secret
            token = nope
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("unknown key should fail")
            .to_string();

        assert!(error.contains("unknown key"));
    }

    #[test]
    fn rejects_missing_required_target_field() {
        let path = write_config(
            "missing-password",
            r#"
            [node-a]
            address = 10.0.0.1
            username = admin
            "#,
        );

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("missing password should fail")
            .to_string();

        assert!(error.contains("password is required"));
    }

    #[test]
    fn rejects_empty_config() {
        let path = write_config("empty", "# no targets\n");

        let error = load_targets(path.to_str().expect("path"))
            .expect_err("empty config should fail")
            .to_string();

        assert!(error.contains("no BMC targets found"));
    }

    fn write_config(name: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "fleet-intelligence-oob-{name}-{}.config",
            std::process::id()
        ));
        fs::write(&path, unindent(content)).expect("write config");
        path
    }

    fn unindent(content: &str) -> String {
        content
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
