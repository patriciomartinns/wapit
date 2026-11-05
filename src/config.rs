use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub phone_number_id: String,
    #[serde(default = "default_display_phone_number")]
    pub display_phone_number: String,
    pub waba_id: String,
    pub access_token: String,
    pub app_secret: String,
    pub webhook_url: String,
    #[serde(default = "default_auto_statuses")]
    pub auto_statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncSettings>,
    /// The token the app expects in Meta's `hub.verify_token` when wapit runs the webhook verification handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_token: Option<String>,
}

impl App {
    /// The app wapit serves when there is no `apps.json`, so it runs out of the box.
    pub fn fallback() -> Self {
        Self {
            id: "default".into(),
            name: "Default app".into(),
            phone_number_id: "100000000000001".into(),
            display_phone_number: "+1 555 0100".into(),
            waba_id: "200000000000001".into(),
            access_token: "wapit-local-token".into(),
            app_secret: "wapit-local-secret".into(),
            webhook_url: "http://localhost:8000/webhook".into(),
            auto_statuses: default_auto_statuses(),
            sync: None,
            verify_token: Some("wapit-verify-token".into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncSettings {
    pub waba_id: String,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub graph_version: Option<String>,
}

fn default_display_phone_number() -> String {
    "15550000000".to_owned()
}

fn default_auto_statuses() -> Vec<String> {
    vec!["sent".to_owned()]
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Template {
    pub name: String,
    pub language: String,
    #[serde(default = "approved")]
    pub status: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_format: Option<String>,
    #[serde(default)]
    pub components: Vec<TemplateComponent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Fields wapit does not model (`sub_category`, `cards`, `otp_type`…), kept so files and syncs round-trip unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn approved() -> String {
    "APPROVED".to_owned()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TemplateComponent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buttons: Option<Vec<TemplateButton>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TemplateButton {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Deserialize)]
struct AppsFile {
    #[serde(default)]
    apps: Vec<App>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TemplatesFile {
    List(Vec<Template>),
    Page { data: Vec<Template> },
}

#[derive(Default)]
struct Loaded {
    apps: Vec<Arc<App>>,
    templates: HashMap<String, Arc<Vec<Template>>>,
    signature: Vec<(PathBuf, Option<SystemTime>)>,
    error: Option<String>,
    using_fallback: bool,
}

/// Reads `apps.json` and `templates/<app>.json` on demand and reloads them when a file changes.
/// A file that fails to parse keeps the last good configuration and reports the error instead of breaking every request.
pub struct ConfigStore {
    dir: PathBuf,
    loaded: Mutex<Loaded>,
    writes: Mutex<()>,
}

static TEMPORARY_FILES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SaveError(pub String);

impl From<SaveError> for String {
    fn from(error: SaveError) -> Self {
        error.0
    }
}

impl ConfigStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            loaded: Mutex::new(Loaded::default()),
            writes: Mutex::new(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn apps_file(&self) -> PathBuf {
        self.dir.join("apps.json")
    }

    pub fn templates_file(&self, app_id: &str) -> PathBuf {
        self.dir.join("templates").join(format!("{app_id}.json"))
    }

    fn signature_for(&self, apps: &[Arc<App>]) -> Vec<(PathBuf, Option<SystemTime>)> {
        std::iter::once(self.apps_file())
            .chain(apps.iter().map(|app| self.templates_file(&app.id)))
            .map(|path| {
                let modified = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok();
                (path, modified)
            })
            .collect()
    }

    fn refresh(&self) -> std::sync::MutexGuard<'_, Loaded> {
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = self.signature_for(&loaded.apps);

        if !loaded.signature.is_empty() && loaded.signature == current {
            return loaded;
        }

        match self.read() {
            Ok((apps, templates, using_fallback)) => {
                loaded.signature = self.signature_for(&apps);
                loaded.using_fallback = using_fallback;
                loaded.apps = apps;
                loaded.templates = templates;
                loaded.error = None;
            }
            Err(error) => {
                if loaded.error.as_deref() != Some(error.as_str()) {
                    tracing::error!("config not reloaded: {error}");
                }

                loaded.signature = current;
                loaded.error = Some(error);
            }
        }

        loaded
    }

    #[allow(clippy::type_complexity)]
    fn read(&self) -> Result<(Vec<Arc<App>>, HashMap<String, Arc<Vec<Template>>>, bool), String> {
        let file = self.apps_file();
        let (apps, using_fallback) = match std::fs::read_to_string(&file) {
            Ok(raw) => {
                let parsed: AppsFile = serde_json::from_str(&raw)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                (parsed.apps, false)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (vec![App::fallback()], true)
            }
            Err(error) => return Err(format!("cannot read {}: {error}", file.display())),
        };

        validate_apps(&apps)?;

        let mut templates = HashMap::new();

        for app in &apps {
            templates.insert(app.id.clone(), Arc::new(self.read_templates(&app.id)?));
        }

        Ok((
            apps.into_iter().map(Arc::new).collect(),
            templates,
            using_fallback,
        ))
    }

    fn read_templates(&self, app_id: &str) -> Result<Vec<Template>, String> {
        let file = self.templates_file(app_id);

        let raw = match std::fs::read_to_string(&file) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("cannot read {}: {error}", file.display())),
        };

        let mut list = match serde_json::from_str(&raw)
            .map_err(|error| format!("{}: {error}", file.display()))?
        {
            TemplatesFile::List(list) | TemplatesFile::Page { data: list } => list,
        };

        for template in &mut list {
            if template.id.is_none() {
                template.id = Some(stable_template_id(&template.name, &template.language));
            }
        }

        Ok(list)
    }

    /// Reads, changes and writes an app's templates under one lock, so concurrent edits never overwrite each other.
    /// The file is written to a unique temporary name first, so a crash never leaves it half written.
    pub fn edit_templates<T, E: From<SaveError>>(
        &self,
        app_id: &str,
        edit: impl FnOnce(&mut Vec<Template>) -> Result<T, E>,
    ) -> Result<T, E> {
        let _writing = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut templates = self.templates_for(app_id).as_ref().clone();
        let result = edit(&mut templates)?;
        let file = self.templates_file(app_id);

        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                SaveError(format!("cannot create {}: {error}", parent.display()))
            })?;
        }

        let json = serde_json::to_string_pretty(&templates)
            .map_err(|error| SaveError(error.to_string()))?;
        let sequence = TEMPORARY_FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = file.with_extension(format!("json.{}.{sequence}.tmp", std::process::id()));

        std::fs::write(&temporary, format!("{json}\n"))
            .map_err(|error| SaveError(format!("cannot write {}: {error}", temporary.display())))?;

        if let Err(error) = std::fs::rename(&temporary, &file) {
            let _ = std::fs::remove_file(&temporary);
            return Err(SaveError(format!("cannot write {}: {error}", file.display())).into());
        }

        self.loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signature
            .clear();

        Ok(result)
    }

    pub fn using_fallback(&self) -> bool {
        self.refresh().using_fallback
    }

    pub fn apps(&self) -> Vec<Arc<App>> {
        self.refresh().apps.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.refresh().error.clone()
    }

    pub fn app(&self, id: &str) -> Option<Arc<App>> {
        self.refresh().apps.iter().find(|app| app.id == id).cloned()
    }

    pub fn app_by_token(&self, token: &str) -> Option<Arc<App>> {
        self.refresh()
            .apps
            .iter()
            .find(|app| app.access_token == token)
            .cloned()
    }

    pub fn templates_for(&self, app_id: &str) -> Arc<Vec<Template>> {
        self.refresh()
            .templates
            .get(app_id)
            .cloned()
            .unwrap_or_default()
    }
}

/// Templates written by hand have no id. Deriving one from name and language keeps it stable across reloads.
fn stable_template_id(name: &str, language: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for byte in name.bytes().chain([0]).chain(language.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }

    format!("{}", 1_000_000_000_000_000 + hash % 9_000_000_000_000_000)
}

fn validate_apps(apps: &[App]) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut tokens = HashSet::new();
    let mut phone_numbers = HashSet::new();

    for (index, app) in apps.iter().enumerate() {
        let required = [
            ("id", &app.id),
            ("name", &app.name),
            ("phone_number_id", &app.phone_number_id),
            ("waba_id", &app.waba_id),
            ("access_token", &app.access_token),
            ("app_secret", &app.app_secret),
            ("webhook_url", &app.webhook_url),
        ];

        for (key, value) in required {
            if value.trim().is_empty() {
                return Err(format!("apps[{index}].{key} must be a non-empty string"));
            }
        }

        if !app
            .id
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_')
        {
            return Err(format!(
                "apps[{index}].id may only use letters, digits, - and _"
            ));
        }

        if !ids.insert(&app.id) {
            return Err(format!(
                "apps[{index}].id {} is used by another app",
                app.id
            ));
        }

        if !tokens.insert(&app.access_token) {
            return Err(format!("apps[{index}].access_token is used by another app"));
        }

        if !phone_numbers.insert(&app.phone_number_id) {
            return Err(format!(
                "apps[{index}].phone_number_id {} is used by another app",
                app.phone_number_id
            ));
        }
    }

    Ok(())
}
