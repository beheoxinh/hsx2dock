fn main() {
    glib_build_tools::compile_resources(
        &["data"],
        "data/resources.gresource.xml",
        "com.github.docker_manager.gresource",
    );
}
