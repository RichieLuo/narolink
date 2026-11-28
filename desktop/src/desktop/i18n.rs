use eframe::egui;
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Language {
    #[default]
    Chinese,
    English,
    Japanese,
}

static MESSAGES: LazyLock<HashMap<String, [String; 2]>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../web/public/i18n.json"))
        .expect("bundled Naro translations")
});

impl Language {
    pub(super) const ALL: [Self; 3] = [Self::Chinese, Self::English, Self::Japanese];

    pub(super) fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
    }

    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
            Self::Japanese => "ja",
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Chinese => "简体中文",
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }

    pub(super) fn apply(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("interface language"), self));
    }

    pub(super) fn in_ui(ui: &egui::Ui) -> Self {
        ui.data(|data| data.get_temp(egui::Id::new("interface language")))
            .unwrap_or_default()
    }

    pub(super) fn text(self, source: &str) -> &str {
        let index = match self {
            Self::Chinese => return source,
            Self::English => 0,
            Self::Japanese => 1,
        };
        MESSAGES
            .get(source)
            .map_or(source, |entry| entry[index].as_str())
    }

    pub(super) fn format(self, source: &str, values: &[(&str, &str)]) -> String {
        let mut remaining = self.text(source);
        let mut result = String::with_capacity(remaining.len());
        while let Some((prefix, rest)) = remaining.split_once('{') {
            let Some((name, suffix)) = rest.split_once('}') else {
                break;
            };
            result.push_str(prefix);
            if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
                result.push_str(value);
            } else {
                result.push('{');
                result.push_str(name);
                result.push('}');
            }
            remaining = suffix;
        }
        result.push_str(remaining);
        result
    }

    pub(super) fn quota_window(self, source: &str) -> Cow<'_, str> {
        if self == Self::Chinese {
            return source.into();
        }
        // Older hosts send display labels with the duration already interpolated.
        let (name, duration) = source.rsplit_once(" · ").unwrap_or(("", source));
        let Some((count, unit)) = duration.split_once(' ') else {
            return self.text(source).into();
        };
        let template = match unit {
            "分钟" => "{count} 分钟",
            "小时" => "{count} 小时",
            "天" => "{count} 天",
            _ => return self.text(source).into(),
        };
        if count.parse::<u64>().is_err() {
            return self.text(source).into();
        }
        let translated = self.format(template, &[("count", count)]);
        if name.is_empty() {
            translated.into()
        } else {
            format!("{} · {translated}", self.text(name)).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Language;

    #[test]
    fn bundled_translations_use_stable_language_codes_and_preserve_interpolated_text() {
        for (language, settings) in Language::ALL.into_iter().zip(["设置", "Settings", "設定"])
        {
            assert!(Language::from_code(language.code()) == Some(language));
            assert_eq!(language.text("设置"), settings);
        }
        assert!(Language::from_code("unknown").is_none());
        assert_eq!(Language::Japanese.text("CLI output"), "CLI output");
        assert_eq!(
            Language::English.format(
                "在 {project} 新建会话（{kind}）",
                &[("project", "{kind}"), ("kind", "Codex")]
            ),
            "New session in {kind} (Codex)"
        );
        assert_eq!(
            Language::Japanese.quota_window("Opus · 7 天"),
            "Opus · 7 日"
        );
    }
}
