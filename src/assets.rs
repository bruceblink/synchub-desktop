use gpui::*;
use gpui_component_assets::Assets as ComponentAssets;

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == "branding/logo-mark.svg" {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../resources/branding/logo-mark.svg"
            ))));
        }
        ComponentAssets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        if path == "branding" {
            return Ok(vec!["branding/logo-mark.svg".into()]);
        }
        ComponentAssets.list(path)
    }
}
