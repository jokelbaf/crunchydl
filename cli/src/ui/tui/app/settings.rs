use crate::ui::tui::*;

impl App {
    pub(crate) fn begin_setting_edit(&mut self) {
        let field = SettingsField::ALL[self.settings_selected];
        if matches!(field, SettingsField::DrmBackend) {
            self.config.drm_backend = match self.config.drm_backend {
                DrmBackend::Auto => DrmBackend::PlayReady,
                DrmBackend::PlayReady => DrmBackend::Widevine,
                DrmBackend::Widevine => DrmBackend::Auto,
            };
            self.save_config();
            return;
        }
        self.edit_buffer = match field {
            SettingsField::OutputDirectory => self.config.output_dir.display().to_string(),
            SettingsField::Filename => self.config.filename.clone(),
            SettingsField::FolderLayout => self.config.output_layout.clone().unwrap_or_default(),
            SettingsField::DrmDevice => self
                .config
                .drm_device
                .as_deref()
                .map_or_else(String::new, |path| path.display().to_string()),
            SettingsField::LicenseEndpoint => String::new(),
            SettingsField::DrmBackend => unreachable!(),
        };
        self.settings_editing = Some(field);
    }

    pub(crate) fn apply_setting_edit(&mut self) {
        let Some(field) = self.settings_editing.take() else {
            return;
        };
        let value = self.edit_buffer.trim().to_string();
        let validation = match field {
            SettingsField::OutputDirectory if value.is_empty() => {
                Err("Output directory cannot be empty")
            }
            SettingsField::Filename => crunchydl::FilenameTemplate::compile(&value)
                .map(|_| ())
                .map_err(|_| "Filename template is invalid"),
            SettingsField::FolderLayout if !value.is_empty() => {
                crunchydl::OutputLayoutTemplate::compile(&value)
                    .map(|_| ())
                    .map_err(|_| "Folder layout template is invalid")
            }
            _ => Ok(()),
        };
        if let Err(message) = validation {
            self.settings_editing = Some(field);
            self.set_notice(NoticeKind::Error, message);
            return;
        }
        match field {
            SettingsField::OutputDirectory => self.config.output_dir = PathBuf::from(value),
            SettingsField::Filename => self.config.filename = value,
            SettingsField::FolderLayout => {
                self.config.output_layout = (!value.is_empty()).then_some(value);
            }
            SettingsField::DrmDevice => {
                self.config.drm_device = (!value.is_empty()).then(|| PathBuf::from(value));
            }
            SettingsField::LicenseEndpoint => {
                self.config.license_endpoint = (!value.is_empty()).then_some(value);
            }
            SettingsField::DrmBackend => {}
        }
        self.edit_buffer.clear();
        self.save_config();
    }

    pub(crate) fn save_config(&mut self) {
        match self.config.save(&self.paths) {
            Ok(()) => self.set_notice(NoticeKind::Success, "Configuration saved"),
            Err(error) => self.set_notice(NoticeKind::Error, error.to_string()),
        }
    }
}
