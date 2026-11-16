# Bundled GPUI Kit themes

The 21 JSON files in this directory are unmodified copies of `themes/*.json`
from [GPUI Kit v0.9.1](https://github.com/longbridge/gpui-kit/tree/39ab849a93b5f15507b969164d49891db8d5b470/themes),
commit `39ab849a93b5f15507b969164d49891db8d5b470`.

They provide 36 light/dark variants in addition to the two defaults embedded in
`gpui-component`. Each file preserves its upstream author and source URL. The
upstream Apache-2.0 license is included as `LICENSE-APACHE`.

`crates/rtools/src/theme.rs` embeds these files with `include_str!` and registers
them before loading user preferences. No runtime theme directory or network
access is required. To update them, use one pinned upstream revision, update
the embedded file list, and run the theme resource and settings tests.
