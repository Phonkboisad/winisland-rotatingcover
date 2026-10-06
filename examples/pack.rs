use winisland_plugin_api::packager::PluginPackager;

fn main() {
    PluginPackager::from_cargo()
        .expect("read plugin metadata from Cargo.toml")
        .build()
        .expect("build WinIsland plugin package");
}
