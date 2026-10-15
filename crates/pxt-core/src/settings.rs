//! Impostazioni utente (`settings.json`). Nessun segreto qui.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;

pub const MIN_FONT_SIZE: u8 = 8;
pub const MAX_FONT_SIZE: u8 = 32;
pub const MIN_SCROLLBACK: u32 = 1_000;
pub const MAX_SCROLLBACK: u32 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    Dark,
    Light,
    SolarizedDark,
    Dracula,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub schema_version: u32,
    pub theme: Theme,
    pub font_family: String,
    pub font_size: u8,
    pub scrollback: u32,
    /// Copia automatica negli appunti quando si seleziona del testo.
    pub copy_on_select: bool,
    /// Il tasto destro incolla (stile PuTTY/MobaXterm).
    pub right_click_paste: bool,
    /// Chiede conferma prima di incollare più righe.
    pub paste_multiline_warning: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: Theme::Dark,
            font_family: "Consolas, \"DejaVu Sans Mono\", monospace".into(),
            font_size: 14,
            scrollback: 10_000,
            copy_on_select: true,
            right_click_paste: true,
            paste_multiline_warning: true,
        }
    }
}

impl Settings {
    /// Riporta i valori nei limiti accettati (un file modificato a mano non deve rompere l'app).
    pub fn sanitized(mut self) -> Self {
        self.font_size = self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.scrollback = self.scrollback.clamp(MIN_SCROLLBACK, MAX_SCROLLBACK);
        if self.font_family.trim().is_empty() {
            self.font_family = Settings::default().font_family;
        }
        self
    }
}

pub struct LocalSettings {
    path: PathBuf,
    lock: Mutex<()>,
}

impl LocalSettings {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Impostazioni salvate, o i default se il file non esiste.
    pub fn load(&self) -> Result<Settings> {
        let _g = self.lock.lock().unwrap();
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str::<Settings>(&text)?.sanitized()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Salva (dopo la validazione) e restituisce i valori effettivamente scritti.
    pub fn save(&self, settings: Settings) -> Result<Settings> {
        let _g = self.lock.lock().unwrap();
        let settings = settings.sanitized();
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, serde_json::to_vec_pretty(&settings)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!("pxt-settings-{}", uuid::Uuid::new_v4()))
            .join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let s = LocalSettings::new(temp_file());
        assert_eq!(s.load().unwrap(), Settings::default());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let path = temp_file();
        let s = LocalSettings::new(&path);
        let custom = Settings {
            theme: Theme::Dracula,
            font_size: 18,
            copy_on_select: false,
            ..Settings::default()
        };
        s.save(custom.clone()).unwrap();
        assert_eq!(LocalSettings::new(&path).load().unwrap(), custom);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn out_of_range_values_are_clamped_and_unknown_fields_ignored() {
        let path = temp_file();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"fontSize": 200, "scrollback": 5, "fontFamily": "  ", "theme": "light", "futuro": 1}"#,
        )
        .unwrap();
        let loaded = LocalSettings::new(&path).load().unwrap();
        assert_eq!(loaded.font_size, MAX_FONT_SIZE);
        assert_eq!(loaded.scrollback, MIN_SCROLLBACK);
        assert_eq!(loaded.font_family, Settings::default().font_family);
        assert_eq!(loaded.theme, Theme::Light);
        assert!(
            loaded.copy_on_select,
            "i campi mancanti prendono il default"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn theme_serializes_kebab_case() {
        let v = serde_json::to_value(Settings {
            theme: Theme::SolarizedDark,
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(v["theme"], "solarized-dark");
    }
}
