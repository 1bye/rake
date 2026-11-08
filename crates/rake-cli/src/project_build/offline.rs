//! Deterministic deployment identities and opt-in immutable service-worker payloads.

use super::{
    Result,
    bundle::{files, html, insert, rewrite},
    json_line, string,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub(super) fn prepare(site: &Path, metadata: &mut Value, offline: bool) -> Result<()> {
    for name in [
        "rake-version.json",
        "rake-service-worker.js",
        "rake-offline.js",
        "rake-builds",
    ] {
        if site.join(name).exists() {
            return Err(format!("{name} is reserved for generated deployment assets").into());
        }
    }
    let mut identity = json!({"protocol":metadata["protocol"],"application_id":metadata["application_id"],"version":metadata["version"],"base_path":metadata["base_path"]});
    let mut digest = Sha256::new();
    digest.update(json_line(&identity)?.as_bytes());
    // The implementation is part of the identity, as web_offline.py was before the port.
    digest.update(include_bytes!("offline.rs"));
    let template = include_str!("../../scripts/web-service-worker.js");
    if offline {
        digest.update(template.as_bytes());
    }
    for path in files(site)? {
        digest.update(super::relative_name(path.strip_prefix(site)?).as_bytes());
        digest.update(b"\0");
        digest.update(Sha256::digest(fs::read(path)?));
    }
    let build = format!("{:x}", digest.finalize());
    identity["build"] = json!(build);
    metadata["build"] = json!(build);
    metadata["offline"] = json!(offline);
    let base = string(metadata, "base_path")?.to_owned();
    let index = site.join("index.html");
    fs::write(
        &index,
        insert(
            &fs::read_to_string(&index)?,
            &format!(
                "<meta name=\"rake-deployment\" content=\"{}\">",
                html(&json_line(&identity)?)
            ),
        )?,
    )?;
    if offline {
        let entries = fs::read_dir(site)?.collect::<std::io::Result<Vec<_>>>()?;
        let relative = format!("rake-builds/{build}/");
        let assets_base = format!("{base}{relative}");
        metadata["assets"] = json!(format!("{assets_base}assets/"));
        metadata["cache"] =
            json!("immutable build assets; network-first navigation; network-only version checks");
        rewrite(site, &base, &assets_base)?;
        for path in files(site)? {
            if path.starts_with(site.join("pkg"))
                || !path
                    .extension()
                    .is_some_and(|ext| ["html", "js", "css"].iter().any(|value| ext == *value))
            {
                continue;
            }
            let text = fs::read_to_string(&path)?.replace(
                &format!("<base href=\"{}\">", html(&base)),
                &format!("<base href=\"{}\">", html(&assets_base)),
            );
            fs::write(path, text)?;
        }
        let destination = site.join(relative);
        fs::create_dir_all(&destination)?;
        for entry in entries {
            fs::rename(entry.path(), destination.join(entry.file_name()))?;
        }
        let document = insert(
            &fs::read_to_string(destination.join("index.html"))?,
            &format!("<script src=\"{assets_base}rake-offline.js\" defer></script>"),
        )?;
        fs::write(&index, &document)?;
        fs::write(destination.join("index.html"), document)?;
        fs::write(
            destination.join("rake-offline.js"),
            format!(
                "(() => {{ const root = {};\nif (!('serviceWorker' in navigator)) {{ document.documentElement.dataset.rakeOffline = 'unavailable'; return; }}\nnavigator.serviceWorker.register(root + 'rake-service-worker.js', {{scope: root, updateViaCache: 'none'}})\n.then(() => navigator.serviceWorker.ready).then(() => {{ document.documentElement.dataset.rakeOffline = 'ready'; }})\n.catch(error => {{ document.documentElement.dataset.rakeOffline = 'failed'; console.warn('Offline setup failed', error); }}); }})();\n",
                json_line(&base)?
            ),
        )?;
        let assets: Vec<_> = files(&destination)?
            .iter()
            .map(|path| {
                let path =
                    super::relative_name(path.strip_prefix(site).expect("payload is inside site"));
                // urllib.quote(safe="/"): retain RFC 3986 unreserved characters and separators.
                let encoded = path
                    .bytes()
                    .map(|byte| {
                        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                            (byte as char).to_string()
                        } else {
                            format!("%{byte:02X}")
                        }
                    })
                    .collect::<String>();
                format!("{base}{encoded}")
            })
            .collect();
        let prefix = format!(
            "rake-offline:{:x}:",
            Sha256::digest(format!("{}{base}", string(metadata, "application_id")?).as_bytes())
        );
        let config = format!(
            "const CACHE={}, PREFIX={}, ROOT={}, INDEX={}, ASSETS={};\n",
            json_line(&format!("{prefix}{build}"))?,
            json_line(&prefix)?,
            json_line(&base)?,
            json_line(&format!("{assets_base}index.html"))?,
            json_line(&assets)?
        );
        fs::write(site.join("rake-service-worker.js"), config + template)?;
    }
    fs::write(
        site.join("rake-version.json"),
        format!(
            "{}\n",
            super::ascii_json(&serde_json::to_string_pretty(&identity)?)
        ),
    )?;
    Ok(())
}
