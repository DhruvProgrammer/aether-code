fn main() {
    #[cfg(target_os = "windows")]
    {
        let _ = embed_resource::compile("resource.rc", embed_resource::NONE);
    }
}
