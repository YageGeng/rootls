//! Icon assets: the default component icons plus the extras rtools uses.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

// Icons outside the default component set: plugin icons and shell buttons.
icon_assets!(
    ExtraIcons,
    [
        Braces,
        Power,
        AppWindow,
        ListIndentIncrease,
        FoldVertical,
        Quote,
        RemoveFormatting,
        CodeXml,
        GitCompare,
        Languages,
        ArrowUpDown,
        ArrowLeftRight,
        Eraser,
        Space,
        Type,
        UnfoldVertical
    ]
);

/// Looks up the extra icons first, then the default component icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    /// Loads an asset from the extras, falling back to the defaults.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    /// Lists assets from both sources.
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ExtraIcons.list(path)?;
        paths.extend(Assets.list(path)?);
        Ok(paths)
    }
}
