use gpui::*;

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub bg: Hsla,
    pub panel: Hsla,
    pub panel_alt: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            bg: Hsla::from(rgb(0xf8fafc)),
            panel: Hsla::from(rgb(0xffffff)),
            panel_alt: Hsla::from(rgb(0xf1f5f9)),
            border: Hsla::from(rgb(0xd9e2ec)),
            text: Hsla::from(rgb(0x172033)),
            muted: Hsla::from(rgb(0x64748b)),
            accent: Hsla::from(rgb(0x2563eb)),
            success: Hsla::from(rgb(0x16a34a)),
            warning: Hsla::from(rgb(0xd97706)),
            danger: Hsla::from(rgb(0xdc2626)),
        }
    }
}

pub fn alpha(mut color: Hsla, alpha: f32) -> Hsla {
    color.a = alpha;
    color
}
