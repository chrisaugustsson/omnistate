fn main() {
    // sqlx::migrate! embeds the files of migrations/ at compile time, so a new
    // or changed migration must rebuild the crate.
    println!("cargo:rerun-if-changed=../../migrations");
}
