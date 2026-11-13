//! What `--image` names, on caches made here. Building an image needs
//! Docker and the network, so it is not a test; `kai vm image build rust`
//! is the command that does it.

use std::path::PathBuf;

use vm::Error;
use vm::image::{IMAGE_FORMAT, ImageInfo, RECIPES, image_id, recipe, resolve};

fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-vm-image-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Puts a made-up image of `recipe` into `cache`: a file and its JSON.
fn put(cache: &std::path::Path, recipe_name: &str) -> (PathBuf, ImageInfo) {
    let recipe = recipe(recipe_name).unwrap();
    let id = image_id(recipe);
    let hex = id.trim_start_matches("sha256:").to_owned();
    let images = cache.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let file = images.join(format!("{hex}.sqfs"));
    std::fs::write(&file, b"an invented image").unwrap();
    let info = ImageInfo {
        format: IMAGE_FORMAT.to_owned(),
        recipe: recipe_name.to_owned(),
        id,
        file: format!("sha256:{}", "4".repeat(64)),
        docker_image: format!("sha256:{}", "5".repeat(64)),
        env: vec!["PATH=/usr/local/cargo/bin:/usr/bin:/bin".to_owned()],
        working_dir: None,
        user: None,
    };
    std::fs::write(
        images.join(format!("{hex}.json")),
        serde_json::to_vec(&info).unwrap(),
    )
    .unwrap();
    (file, info)
}

#[test]
fn kai_has_the_rust_recipe() {
    assert!(RECIPES.iter().any(|recipe| recipe.name == "rust"));
}

#[test]
fn a_recipe_that_is_not_built_names_the_command_that_builds_it() {
    let cache = dir("not-built");
    let err = resolve("rust", &cache).unwrap_err();
    assert!(
        matches!(err, Error::NotBuilt(ref name) if name == "rust"),
        "{err}"
    );
    assert!(
        err.to_string().contains("`kai vm image build rust`"),
        "{err}"
    );
}

#[test]
fn a_built_recipe_and_its_id_name_the_same_image() {
    let cache = dir("built");
    let (file, info) = put(&cache, "rust");

    let (by_name, found) = resolve("rust", &cache).unwrap();
    assert_eq!(by_name, file);
    assert_eq!(found.as_ref(), Some(&info));

    let (by_id, found) = resolve(&info.id, &cache).unwrap();
    assert_eq!(by_id, file);
    assert_eq!(found, Some(info));
}

#[test]
fn an_image_without_its_file_is_not_built() {
    let cache = dir("no-file");
    let (file, _) = put(&cache, "rust");
    std::fs::remove_file(file).unwrap();
    assert!(matches!(resolve("rust", &cache), Err(Error::NotBuilt(_))));
}

#[test]
fn an_unknown_id_is_refused_and_anything_else_is_a_file() {
    let cache = dir("other");
    let unknown = format!("sha256:{}", "6".repeat(64));
    assert!(resolve(&unknown, &cache).is_err());

    let (file, info) = resolve("images/test.sqfs", &cache).unwrap();
    assert_eq!(file, PathBuf::from("images/test.sqfs"));
    assert_eq!(info, None);
}
