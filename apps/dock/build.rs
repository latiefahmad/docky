fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/docky.ico");
    let mut resources = winresource::WindowsResource::new();
    resources.set_icon("../../assets/brand/docky.ico");
    // Code signing checks that every signed file names the same product.
    resources.set("ProductName", "Docky");
    resources.set("FileDescription", "Docky dock");
    if let Err(error) = resources.compile() {
        // The icon is cosmetic: a missing resource compiler must not block the build.
        println!("cargo:warning=no se pudo incrustar el icono: {error}");
    }
}
