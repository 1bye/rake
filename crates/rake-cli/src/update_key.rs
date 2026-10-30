//! Update-key enrollment with public-only terminal output.
use rake_dist::UpdateSigningKey;
use std::path::{Path, PathBuf};

pub(crate) fn run(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let action = arguments.first().map(String::as_str).ok_or("use update-key generate|public --private-key PATH --public-key PATH, or --keychain-service NAME --keychain-account NAME")?;
    if !matches!(action, "generate" | "public") {
        return Err("expected update-key generate or public".into());
    }
    let mut file = None;
    let mut variable = None;
    let mut service = None;
    let mut account = None;
    let mut public = None;
    let mut args = arguments[1..].iter();
    while let Some(arg) = args.next() {
        if arg == "--json" {
            continue;
        }
        let value = args
            .next()
            .ok_or("update-key option needs a value")?
            .clone();
        let slot = match arg.as_str() {
            "--private-key" => &mut file,
            "--env" => &mut variable,
            "--keychain-service" => &mut service,
            "--keychain-account" => &mut account,
            "--public-key" => &mut public,
            _ => return Err(format!("unsupported update-key argument: {arg}").into()),
        };
        if slot.replace(value).is_some() {
            return Err("duplicate update-key argument".into());
        }
    }
    let reference = match (file, variable, service, account) {
        (Some(path), None, None, None) => UpdateSigningKey::File { path: path.into() },
        (None, Some(variable), None, None) => UpdateSigningKey::Environment { variable },
        (None, None, Some(service), Some(account)) => {
            UpdateSigningKey::Keychain { service, account }
        }
        _ => {
            return Err(
                "select one private-key file, env variable or Keychain service/account".into(),
            );
        }
    };
    let output = PathBuf::from(public.ok_or("--public-key output path is required")?);
    if action == "generate" {
        rake_dist::generate_update_key(&reference, &output)?;
    } else {
        rake_dist::export_update_public_key(&reference, Path::new("."), &output)?;
    }
    println!("{}", serde_json::json!({"public_key":output}));
    Ok(())
}
