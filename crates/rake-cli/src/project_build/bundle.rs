//! Production Web deployment, archive and manifest generation owned by the native adapter.

use super::{Args, Processes, Project, Result, json_line, offline, string};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn files(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(root)? {
            let path = entry?.path();

            if path.is_dir() {
                walk(&path, found)?;
            } else if path.is_file() {
                found.push(path);
            }
        }

        Ok(())
    }
    let mut found = Vec::new();

    walk(root, &mut found)?;
    found.sort();

    Ok(found)
}

pub(super) fn hash(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();

    std::io::copy(&mut fs::File::open(path)?, &mut digest)?;

    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

pub(super) fn insert(document: &str, content: &str) -> Result<String> {
    let head = regex::Regex::new(r"(?i)<head\b[^>]*>")?;
    let root = regex::Regex::new(r"(?i)<html\b[^>]*>")?;

    let end = head
        .find(document)
        .or_else(|| root.find(document))
        .ok_or("Web document requires an html or head element")?
        .end();

    Ok(format!(
        "{}{}{}",
        &document[..end],
        content,
        &document[end..]
    ))
}

pub(super) fn deployment(project: &Project, args: &Args, site: &Path) -> Result<Value> {
    let config = &project.package["metadata"]["rake"]["bundle"];
    let web = &config["web"];

    let base = args
        .base_path
        .as_deref()
        .or(web["base_path"].as_str())
        .unwrap_or("/");

    let routing = args
        .routing
        .as_deref()
        .or(web["routing"].as_str())
        .unwrap_or("path");

    let identifier = config["identifier"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "dev.rake.{}",
                project.package["name"]
                    .as_str()
                    .unwrap_or_default()
                    .replace('_', "-")
            )
        });

    let index = site.join("index.html");
    let document = fs::read_to_string(&index)?;

    for (name, message) in [
        (
            "rake-deployment.json",
            "rake-deployment.json is reserved for generated deployment metadata",
        ),
        ("__rake", "__rake is reserved for development tooling"),
    ] {
        if site.join(name).exists() {
            return Err(message.into());
        }
    }

    if regex::Regex::new(r"rake-dev-bridge|/__rake/|rake-web-config")?.is_match(&document) {
        return Err("Web input contains development or existing deployment metadata".into());
    }

    if regex::Regex::new(r"(?i)<base\b")?.is_match(&document) {
        return Err("remove the custom base element; rake bundle owns the deployment base".into());
    }

    rewrite(site, "/", base)?;
    let metadata = json_line(&DeploymentConfig {
        base_path: base,
        routing,
    })?;

    let insertion = format!(
        "<base href=\"{}\"><meta name=\"rake-web-config\" content=\"{}\">",
        html(base),
        html(&metadata)
    );

    fs::write(&index, insert(&fs::read_to_string(&index)?, &insertion)?)?;
    let mut metadata = json!({"protocol":1,"application_id":identifier,"version":project.package["version"],"base_path":base,"routing":routing,"assets":format!("{base}assets/"),"wasm_mime":"application/wasm","development_control":false,"cache":"revalidate; filenames are not content hashed"});

    offline::prepare(
        site,
        &mut metadata,
        web["offline"].as_bool().unwrap_or(false),
    )?;
    fs::write(
        site.join("rake-deployment.json"),
        format!(
            "{}\n",
            super::ascii_json(&serde_json::to_string_pretty(&metadata)?)
        ),
    )?;

    Ok(metadata)
}

#[derive(serde::Serialize)]
struct DeploymentConfig<'a> {
    base_path: &'a str,
    routing: &'a str,
}

pub(super) fn rewrite(site: &Path, base: &str, replacement: &str) -> Result<()> {
    let pattern = regex::Regex::new(&format!(r#"(['"]){}(pkg|assets)/"#, regex::escape(base)))?;

    for path in files(site)? {
        if path.starts_with(site.join("pkg"))
            || !path
                .extension()
                .is_some_and(|ext| ["html", "js", "css"].iter().any(|value| ext == *value))
        {
            continue;
        }

        let text = fs::read_to_string(&path)?;

        let text = pattern.replace_all(&text, |capture: &regex::Captures<'_>| {
            format!("{}{replacement}{}/", &capture[1], &capture[2])
        });

        fs::write(path, text.as_bytes())?;
    }

    Ok(())
}

fn zip_time(modified: std::time::SystemTime) -> Result<zip::DateTime> {
    let seconds: libc::time_t = modified
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?;

    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();

    #[cfg(unix)]
    // SAFETY: both pointers reference live correctly sized objects; success initializes local.
    let valid = !unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null();

    #[cfg(windows)]
    // SAFETY: the CRT initializes the correctly sized output tm on success.
    let valid = unsafe { libc::localtime_s(local.as_mut_ptr(), &seconds) } == 0;

    if !valid {
        return Err("ZIP timestamp is out of range".into());
    }

    // SAFETY: the platform localtime conversion succeeded and initialized tm.
    let local = unsafe { local.assume_init() };

    Ok(zip::DateTime::from_date_and_time(
        (local.tm_year + 1900).try_into()?,
        (local.tm_mon + 1).try_into()?,
        local.tm_mday.try_into()?,
        local.tm_hour.try_into()?,
        local.tm_min.try_into()?,
        local.tm_sec.try_into()?,
    )?)
}

pub(super) fn distribution(
    project: &Project,
    args: &Args,
    processes: &Processes<'_>,
    output: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(output)?;
    super::permissions::set(output, 0o755)?;
    let site = output.join("site");

    project.build(args, processes, &site)?;
    let deployment = deployment(project, args, &site)?;

    let name = format!(
        "{}-{}-web.zip",
        string(&project.package, "name")?,
        string(&project.package, "version")?
    );

    let archive = output.join(&name);
    let payload = files(&site)?;
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive)?);

    for path in &payload {
        processes.check_cancelled()?;
        zip.start_file(
            super::relative_name(path.strip_prefix(&site)?),
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(zip_time(fs::metadata(path)?.modified()?)?)
                .unix_permissions(super::permissions::archive_mode(path)?),
        )?;
        std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
    }

    zip.finish()?.sync_all()?;
    let base = string(&deployment, "base_path")?;

    let fallback = if deployment["routing"] == "path" {
        format!("try_files $uri $uri/ {base}index.html;")
    } else {
        "try_files $uri $uri/ =404;".into()
    };

    let redirect = if base == "/" {
        String::new()
    } else {
        format!(
            "location = {} {{ return 308 {base}; }}\n",
            &base[..base.len() - 1]
        )
    };

    fs::write(
        output.join("nginx.conf.example"),
        format!(
            "# Install site/ contents below /srv/www{base}; include the server's standard MIME types.\n{redirect}# Retain previous rake-builds/ directories while old tabs may still request their assets.\nlocation {base}rake-builds/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'public, max-age=31536000, immutable'; }}\nlocation {base}pkg/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'no-cache'; }}\nlocation {base}assets/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'no-cache'; }}\nlocation {base} {{ root /srv/www; index index.html; {fallback} add_header Cache-Control 'no-cache'; }}\n"
        ),
    )?;
    let files: Vec<_> = payload.iter().map(|path| Ok(json!({"path":super::relative_name(path.strip_prefix(output)?),"bytes":fs::metadata(path)?.len(),"sha256":hash(path)?}))).collect::<Result<_>>()?;
    let config = &project.package["metadata"]["rake"]["bundle"];
    let checksum = hash(&archive)?;
    let manifest = json!({"protocol":1,"package":project.package["name"],"application_id":deployment["application_id"],"name":config.get("name").unwrap_or(&project.package["name"]),"version":project.package["version"],"target":"web","platform":"web","architecture":"wasm32","profile":if args.release {"release"} else {"debug"},"signing":"not_applicable","notarized":false,"artifact":"site","deployment":deployment,"archive":{"path":name,"bytes":fs::metadata(&archive)?.len(),"sha256":checksum},"files":files});

    fs::write(
        output.join("bundle.json"),
        format!(
            "{}\n",
            super::ascii_json(&serde_json::to_string_pretty(&manifest)?)
        ),
    )?;
    fs::write(output.join("SHA256SUMS"), format!("{checksum}  {name}\n"))?;

    Ok(output.join("bundle.json"))
}

pub(super) fn report(output: &Path, artifact: &Path, json: bool) -> Result<()> {
    let manifest: Value = serde_json::from_slice(&fs::read(artifact)?)?;
    let site = output.join(string(&manifest, "artifact")?);
    let archive = output.join(string(&manifest["archive"], "path")?);

    if json {
        println!(
            "{}",
            json_line(
                &json!({"artifact":site,"archive":archive,"manifest":artifact,"sha256":manifest["archive"]["sha256"],"profile":manifest["profile"],"signing":manifest["signing"]})
            )?
        );
    } else {
        println!(
            "Bundled {}\nArchive: {}\nManifest: {}\nDeployment: {} ({})",
            site.display(),
            archive.display(),
            artifact.display(),
            string(&manifest["deployment"], "base_path")?,
            string(&manifest["deployment"], "routing")?
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Covers: online/offline deployment layout, identity, reserved inputs and namespace rewriting.
    //! Real: deployment, hashing and filesystem. Doubles: project metadata and Wasm bytes.
    //! Requires: none.
    use super::*;

    fn project(site: &Path, offline: bool) -> Project {
        Project {
            package: json!({"name":"example","version":"1.0.0","metadata":{"rake":{"bundle":{"identifier":"dev.example.tools","web":{"base_path":"/tools/","offline":offline}}}}}),
            target: "app".into(),
            root: site.into(),
            target_directory: site.join("target"),
            web_dir: site.into(),
            assets: None,
            bindgen: None,
            watch_roots: vec![],
        }
    }

    fn generate(site: &Path, offline: bool, payload: &str) -> Value {
        fs::create_dir_all(site.join("pkg")).unwrap();
        fs::write(site.join("pkg/app.js"), payload).unwrap();
        fs::write(site.join("pkg/app_bg.wasm"), format!("wasm{payload}")).unwrap();
        fs::write(site.join("index.html"),"<html><head></head><script type=\"module\">import init from \"/pkg/app.js\";init();</script></html>").unwrap();
        super::super::favicon::install(site).unwrap();

        deployment(&project(site, offline), &Args::default(), site).unwrap()
    }

    #[test]
    fn offline_builds_pin_assets_and_keep_router_root_separate() {
        let temp = tempfile::tempdir().unwrap();
        let old = generate(&temp.path().join("old"), true, "old");
        let same = generate(&temp.path().join("same"), true, "old");
        let new = generate(&temp.path().join("new"), true, "new");

        assert_eq!(old["build"], same["build"]);
        assert_ne!(old["build"], new["build"]);
        let build = old["build"].as_str().unwrap();
        let prefix = format!("/tools/rake-builds/{build}/");
        let site = temp.path().join("old");
        let index = fs::read_to_string(site.join("index.html")).unwrap();

        for fragment in [
            format!("<base href=\"{prefix}\">"),
            format!("{prefix}pkg/app.js"),
            format!("href=\"{prefix}assets/rake-icon.svg\""),
            format!("src=\"{prefix}rake-offline.js\""),
            "name=\"rake-deployment\"".into(),
        ] {
            assert!(index.contains(&fragment), "{index}");
        }

        assert!(!site.join("pkg").exists());
        let payload = site.join(format!("rake-builds/{build}"));

        assert_eq!(
            fs::read_to_string(payload.join("pkg/app.js")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(payload.join("index.html")).unwrap(),
            index
        );
        let sw = fs::read_to_string(site.join("rake-service-worker.js")).unwrap();

        assert!(sw.contains(&format!("{prefix}pkg/app_bg.wasm")));
        assert!(!sw.contains("self.skipWaiting(") && !sw.contains("clients.claim("));
        let identity: Value =
            serde_json::from_slice(&fs::read(site.join("rake-version.json")).unwrap()).unwrap();

        assert_eq!(identity["base_path"], "/tools/");
        assert_eq!(identity["build"], old["build"]);
    }

    #[test]
    fn online_bundles_have_version_checks_without_changing_asset_layout() {
        let temp = tempfile::tempdir().unwrap();
        let metadata = generate(temp.path(), false, "old");

        assert_eq!(metadata["offline"], false);
        assert!(temp.path().join("pkg/app.js").is_file());
        assert!(temp.path().join("rake-version.json").is_file());
        assert!(!temp.path().join("rake-service-worker.js").exists());
    }

    #[test]
    fn generated_names_and_development_metadata_cannot_collide_with_inputs() {
        for name in [
            "rake-deployment.json",
            "__rake",
            "rake-version.json",
            "rake-service-worker.js",
            "rake-offline.js",
            "rake-builds",
        ] {
            let temp = tempfile::tempdir().unwrap();

            fs::write(temp.path().join("index.html"), "<html><head></head></html>").unwrap();
            fs::write(temp.path().join(name), "owned").unwrap();
            let error =
                deployment(&project(temp.path(), true), &Args::default(), temp.path()).unwrap_err();

            assert!(error.to_string().contains("reserved"), "{error}");
            assert_eq!(fs::read_to_string(temp.path().join(name)).unwrap(), "owned");
        }

        for document in [
            "<html><head><base href='/old/'></head></html>",
            "<html><head>rake-dev-bridge</head></html>",
            "<html><head>/__rake/</head></html>",
            "<html><head>rake-web-config</head></html>",
        ] {
            let temp = tempfile::tempdir().unwrap();

            fs::write(temp.path().join("index.html"), document).unwrap();
            assert!(
                deployment(&project(temp.path(), false), &Args::default(), temp.path()).is_err()
            );
        }
    }

    #[test]
    fn deployment_overrides_rewrite_only_managed_namespaces_and_skip_bindings() {
        let temp = tempfile::tempdir().unwrap();

        fs::create_dir(temp.path().join("pkg")).unwrap();
        let js = "const a='/assets/image'; const b=\"/pkg/app.js\"; const api='/api/notes';";

        fs::write(temp.path().join("pkg/app.js"), js).unwrap();
        fs::write(temp.path().join("loader.js"), js).unwrap();
        fs::write(temp.path().join("index.html"), "<HTML><HEAD></HEAD></HTML>").unwrap();
        let args = Args {
            base_path: Some("/override/".into()),
            routing: Some("hash".into()),
            ..Args::default()
        };

        let metadata = deployment(&project(temp.path(), false), &args, temp.path()).unwrap();

        assert_eq!(metadata["base_path"], "/override/");
        assert_eq!(metadata["routing"], "hash");
        assert_eq!(
            fs::read_to_string(temp.path().join("pkg/app.js")).unwrap(),
            js
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("loader.js")).unwrap(),
            "const a='/override/assets/image'; const b=\"/override/pkg/app.js\"; const api='/api/notes';"
        );
    }
}
