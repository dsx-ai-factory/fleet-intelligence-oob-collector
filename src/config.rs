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
