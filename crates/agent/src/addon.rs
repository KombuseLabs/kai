//! Add-ons: files of a publisher's repository at one commit, each pinned by
//! its SHA-256, laid read-only under `/opt/kai/addons/<name>/` in a layer
//! of their own, which lies beside an agent's layer over the image.
//! [`PONYTAIL`] is the first: one skill and its licence, laid out as a
//! local plugin of Claude Code, whose manifest kai writes. An add-on is a
//! set of files, not a program: nothing of it runs on the host, and whether
//! and how an agent loads it is not this module's business.
//!
//! [`fetch`] is what `kai agent fetch --addon <name>` calls. An add-on's
//! layer is made the way an agent's is ([`crate::layer`]), under the same
//! locks, checks, quarantine and recovery: each pinned file downloaded from
//! its commit and checked against its pin, each file kai writes taken from
//! kai itself, all staged with mode 0644, packed with
//! the same `mksquashfs` options (every file root's, every time stamp 0),
//! read back, its JSON ([`ADDON_FORMAT`]) written last and the entry
//! published by one rename, under [`ADDONS_DIR`] in kai's VM cache.
//!
//! The JSON is the add-on's record:
//!
//! - **The key** is the SHA-256 of the add-on's identity as canonical JSON
//!   (keys sorted, no white space): the format, the name and directory,
//!   the repository, commit and licence, each pinned file (its path in the
//!   repository and in the guest, its SHA-256 and length), each file kai
//!   writes (its path, SHA-256 and length), the packing and `mksquashfs`'s
//!   version. A change of any of them is another entry.
//! - **Two digests, kept apart.** The inventory's names what the layer
//!   holds: every directory and file, with its mode, owner, length and
//!   SHA-256, one line each in order of the path, as the layer is read
//!   back. The layer's names its bytes. Another `mksquashfs` may pack the
//!   same inventory into other bytes, and the key would then differ too.

use std::path::Path;

use helpers::hash::sha256_bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::layer::{
    self, Content, Error, Fetched, PACKING, Packed, Plan, Record, Source, components_under,
};

/// The directory of the guest under which every add-on lies, each in a
/// directory of its name; an add-on's layer adds files there and nowhere
/// else.
pub const ADDONS_PREFIX: &str = "/opt/kai/addons/";

/// The directory of the add-ons' entries in kai's VM cache.
pub const ADDONS_DIR: &str = "addons";

/// The format of an add-on layer's JSON, and the first part of its key.
pub const ADDON_FORMAT: &str = "kai-addon-layer/1";

/// An add-on: files of one repository at one commit, and the licence they
/// come under.
#[derive(Clone, Copy, Debug)]
pub struct Addon {
    /// Its name: lower-case letters, digits and `-`. Its directory in the
    /// guest is [`ADDONS_PREFIX`] followed by the name ([`Addon::dir`]).
    pub name: &'static str,
    /// The publisher's repository, as a URL.
    pub repository: &'static str,
    /// The commit every file is taken from: a full SHA-1 in lower-case
    /// hex, never a branch or a tag.
    pub commit: &'static str,
    /// The licence the repository grants at that commit, as an SPDX
    /// identifier, as a person read it in the text at `licence_file`.
    pub licence: &'static str,
    /// Where that text lies in the guest: the path of one of `files`.
    pub licence_file: &'static str,
    /// The files, each pinned.
    pub files: &'static [AddonFile],
    /// The files kai writes beside them, such as a plugin's manifest.
    pub written: &'static [Written],
}

/// A file kai writes into an add-on's directory, its bytes compiled into
/// kai: what makes the publisher's files usable as they are, never a
/// change of them.
#[derive(Clone, Copy, Debug)]
pub struct Written {
    /// Where it lies in the guest: under the add-on's directory.
    pub path: &'static str,
    /// Its bytes.
    pub bytes: &'static [u8],
}

/// One file of an add-on.
#[derive(Clone, Copy, Debug)]
pub struct AddonFile {
    /// Its path in the repository at the commit.
    pub source: &'static str,
    /// Where it lies in the guest: under the add-on's directory.
    pub path: &'static str,
    /// The download, from that commit, with its SHA-256: the file itself,
    /// no member of an archive, and not a program.
    pub artefact: vm::Artefact,
    /// Its length in bytes.
    pub length: u64,
}

impl Addon {
    /// Its directory in the guest, `/opt/kai/addons/<name>`.
    pub fn dir(&self) -> String {
        format!("{ADDONS_PREFIX}{}", self.name)
    }
}

/// Ponytail's skill `ponytail` and the repository's licence, MIT, at the
/// head of `main` on 29 October 2026 (read 13 November 2026). The skill is
/// the one file of `skills/ponytail/` and names no other; the repository's
/// other skills, hooks and plugin manifests are not taken. Each file is
/// taken from the commit's raw URL, not from an archive GitHub makes on
/// request.
///
/// The add-on's directory is a local plugin of Claude Code: the skill at
/// `skills/ponytail/SKILL.md`, as in the repository, and a manifest kai
/// writes, [`PONYTAIL_PLUGIN`], at `.claude-plugin/plugin.json`. The
/// repository's own manifest names its hooks, which kai does not take.
pub const PONYTAIL: Addon = Addon {
    name: "ponytail",
    repository: "https://github.com/DietrichGebert/ponytail",
    commit: "96608e078b2d5ea607c5c3b8b5a54d1c1fba6846",
    licence: "MIT",
    licence_file: "/opt/kai/addons/ponytail/LICENSE",
    files: &[
        AddonFile {
            source: "skills/ponytail/SKILL.md",
            path: "/opt/kai/addons/ponytail/skills/ponytail/SKILL.md",
            artefact: vm::Artefact {
                name: "Ponytail's skill at 96608e0",
                file: "ponytail-96608e078b2d5ea607c5c3b8b5a54d1c1fba6846-SKILL.md",
                url: "https://raw.githubusercontent.com/DietrichGebert/ponytail/96608e078b2d5ea607c5c3b8b5a54d1c1fba6846/skills/ponytail/SKILL.md",
                download_sha256: "9badd22a32e19cb884e46dfa8d3aae82bb81dfa5484105b8b4f0262df823a509",
                member: None,
                sha256: "9badd22a32e19cb884e46dfa8d3aae82bb81dfa5484105b8b4f0262df823a509",
                executable: false,
            },
            length: 3496,
        },
        AddonFile {
            source: "LICENSE",
            path: "/opt/kai/addons/ponytail/LICENSE",
            artefact: vm::Artefact {
                name: "Ponytail's licence at 96608e0",
                file: "ponytail-96608e078b2d5ea607c5c3b8b5a54d1c1fba6846-LICENSE",
                url: "https://raw.githubusercontent.com/DietrichGebert/ponytail/96608e078b2d5ea607c5c3b8b5a54d1c1fba6846/LICENSE",
                download_sha256: "fb1bc6909ac3ef82d5c22106e32ef682b0cff66788fa915fb9b53b15c9d2f3ab",
                member: None,
                sha256: "fb1bc6909ac3ef82d5c22106e32ef682b0cff66788fa915fb9b53b15c9d2f3ab",
                executable: false,
            },
            length: 1071,
        },
    ],
    written: &[Written {
        path: "/opt/kai/addons/ponytail/.claude-plugin/plugin.json",
        bytes: PONYTAIL_PLUGIN,
    }],
};

/// The manifest kai writes for Ponytail's plugin: its name, which the
/// skill's command `ponytail:ponytail` takes, a description and a version
/// of kai's own; no hooks, commands or settings.
pub const PONYTAIL_PLUGIN: &[u8] = br#"{"name":"ponytail","description":"Ponytail's skill ponytail, pinned and packed by kai","version":"0.0.0"}
"#;

/// Every add-on kai knows, by name.
pub const ADDONS: &[Addon] = &[PONYTAIL];

/// The add-on called `name`, if kai knows one.
pub fn named(name: &str) -> Option<&'static Addon> {
    ADDONS.iter().find(|addon| addon.name == name)
}

/// One file in an add-on's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileInfo {
    /// Its path in the repository at the commit.
    pub source: String,
    /// Where it lies in the guest.
    pub path: String,
    /// Its SHA-256, `sha256:<hex>`.
    pub sha256: String,
    /// Its length in bytes.
    pub length: u64,
}

/// One file kai writes, in an add-on's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WrittenInfo {
    /// Where it lies in the guest.
    pub path: String,
    /// Its SHA-256, `sha256:<hex>`.
    pub sha256: String,
    /// Its length in bytes.
    pub length: u64,
}

/// An add-on layer's record: its JSON, beside it, written last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddonInfo {
    /// [`ADDON_FORMAT`].
    pub format: String,
    /// The entry's key, `sha256:<hex>`: the identity, every field here but
    /// the key, the inventory and the layer's two.
    pub key: String,
    /// The add-on's name.
    pub name: String,
    /// Its directory in the guest.
    pub dir: String,
    /// The publisher's repository.
    pub repository: String,
    /// The commit its files are taken from.
    pub commit: String,
    /// The licence, an SPDX identifier.
    pub licence: String,
    /// Where the licence's text lies in the guest.
    pub licence_file: String,
    /// The files, as pinned.
    pub files: Vec<FileInfo>,
    /// The files kai writes.
    pub written: Vec<WrittenInfo>,
    /// SHA-256 of the layer's inventory, `sha256:<hex>`: what it holds.
    pub inventory: String,
    /// SHA-256 of the layer, `sha256:<hex>`: its bytes.
    pub layer: String,
    /// The layer's length in bytes.
    pub layer_len: u64,
    /// `mksquashfs`'s options, those of an agent's layer.
    pub packing: Vec<String>,
    /// `mksquashfs`'s version.
    pub mksquashfs: String,
}

impl Record for AddonInfo {
    fn layer(&self) -> (&str, u64) {
        (&self.layer, self.layer_len)
    }
}

/// Puts `addon` into the cache under `cache` as a layer, unless an entry
/// there checks out, and says what it did. Downloads only the files the
/// cache does not hold, checked; a pin that is not one kai can fetch is
/// refused before anything is downloaded. Needs `mksquashfs` and
/// `unsquashfs` (squashfs-tools).
pub async fn fetch(addon: &Addon, cache: &Path) -> Result<Fetched<AddonInfo>, Error> {
    check(addon)?;
    let plan = plan(addon, &layer::mksquashfs_version().await?)?;
    layer::publish(&plan, cache, |_| {}).await
}

/// The add-on's layer when the cache holds it and it checks out, hashed
/// afresh; never downloads or builds.
pub async fn cached(addon: &Addon, cache: &Path) -> Result<Fetched<AddonInfo>, Error> {
    check(addon)?;
    let plan = plan(addon, &layer::mksquashfs_version().await?)?;
    layer::find(&plan, cache).await.ok_or_else(|| {
        Error::NotFetched(format!(
            "no valid layer of the add-on {}; run `kai agent fetch --addon {}` first",
            addon.name, addon.name
        ))
    })
}

/// Refuses a pin kai cannot fetch as it is: a name that is no plain name,
/// a commit that is no full SHA-1, a file outside the add-on's directory,
/// twice, a member of an archive or a program, or a licence that is not
/// one of the files.
fn check(addon: &Addon) -> Result<(), Error> {
    let pin = |why: String| Err(Error::Pin(format!("the add-on `{}`: {why}", addon.name)));
    let name = addon.name;
    if name.is_empty()
        || name.len() > 64
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return pin("its name is not lower-case letters, digits and `-`".to_owned());
    }
    if addon.commit.len() != 40
        || !addon
            .commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return pin("its commit is not a full SHA-1 in lower-case hex".to_owned());
    }
    if addon.licence.is_empty() || addon.files.is_empty() {
        return pin("it names no licence or no file".to_owned());
    }
    let prefix = format!("{}/", addon.dir());
    let mut paths = std::collections::BTreeSet::new();
    for file in addon.files {
        components_under(file.path, &prefix)?;
        if !paths.insert(file.path) {
            return pin(format!("`{}` twice", file.path));
        }
        let artefact = &file.artefact;
        if artefact.member.is_some()
            || artefact.executable
            || artefact.sha256 != artefact.download_sha256
        {
            return pin(format!(
                "`{}` is not one plain file as downloaded",
                file.path
            ));
        }
    }
    if !paths.contains(addon.licence_file) {
        return pin("its licence's text is not one of its files".to_owned());
    }
    for file in addon.written {
        components_under(file.path, &prefix)?;
        if !paths.insert(file.path) {
            return pin(format!("`{}` twice", file.path));
        }
    }
    Ok(())
}

/// The add-on's entry, for a pin [`check`] let through.
fn plan<'a>(addon: &'a Addon, mksquashfs: &str) -> Result<Plan<'a, AddonInfo>, Error> {
    let contents: Vec<Content<'_>> = addon
        .files
        .iter()
        .map(|file| Content {
            path: file.path.to_owned(),
            mode: 0o644,
            sha256: file.artefact.sha256.to_owned(),
            length: file.length,
            source: Source::Download(&file.artefact),
        })
        .chain(addon.written.iter().map(|file| Content {
            path: file.path.to_owned(),
            mode: 0o644,
            sha256: sha256_bytes(file.bytes),
            length: file.bytes.len() as u64,
            source: Source::Inline(file.bytes),
        }))
        .collect();
    let inventory = layer::inventory(&contents)?;
    let files: Vec<FileInfo> = addon
        .files
        .iter()
        .map(|file| FileInfo {
            source: file.source.to_owned(),
            path: file.path.to_owned(),
            sha256: format!("sha256:{}", file.artefact.sha256),
            length: file.length,
        })
        .collect();
    let written: Vec<WrittenInfo> = addon
        .written
        .iter()
        .map(|file| WrittenInfo {
            path: file.path.to_owned(),
            sha256: format!("sha256:{}", sha256_bytes(file.bytes)),
            length: file.bytes.len() as u64,
        })
        .collect();
    let packing: Vec<String> = PACKING.iter().map(|&option| option.to_owned()).collect();
    // serde_json's maps are sorted by key: the text is canonical.
    let identity = json!({
        "format": ADDON_FORMAT,
        "name": addon.name,
        "dir": addon.dir(),
        "repository": addon.repository,
        "commit": addon.commit,
        "licence": addon.licence,
        "licence_file": addon.licence_file,
        "files": files,
        "written": written,
        "packing": packing,
        "mksquashfs": mksquashfs,
    });
    let expected = AddonInfo {
        format: ADDON_FORMAT.to_owned(),
        key: format!("sha256:{}", sha256_bytes(identity.to_string().as_bytes())),
        name: addon.name.to_owned(),
        dir: addon.dir(),
        repository: addon.repository.to_owned(),
        commit: addon.commit.to_owned(),
        licence: addon.licence.to_owned(),
        licence_file: addon.licence_file.to_owned(),
        files,
        written,
        inventory,
        layer: String::new(),
        layer_len: 0,
        packing,
        mksquashfs: mksquashfs.to_owned(),
    };
    let record = expected.clone();
    Ok(Plan {
        dir: ADDONS_DIR,
        key: expected.key.clone(),
        contents,
        mismatch: Box::new(move |info| mismatch(&expected, info)),
        record: Box::new(move |packed: &Packed| AddonInfo {
            inventory: packed.inventory.clone(),
            layer: packed.layer.clone(),
            layer_len: packed.layer_len,
            ..record.clone()
        }),
    })
}

/// Why `info` is not the record `expected` stands for, or `None`: the
/// first field that differs, the layer's hash and length aside, which the
/// cache checks against the layer itself.
fn mismatch(expected: &AddonInfo, info: &AddonInfo) -> Option<String> {
    let (Ok(Value::Object(want)), Ok(Value::Object(found))) =
        (serde_json::to_value(expected), serde_json::to_value(info))
    else {
        return Some("it does not serialize".to_owned());
    };
    want.iter()
        .filter(|(field, _)| !matches!(field.as_str(), "layer" | "layer_len"))
        .find(|(field, value)| found.get(*field) != Some(value))
        .and_then(|(field, value)| layer::differs(&format!("`{field}`"), &found.get(field), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(addon: &Addon) -> AddonInfo {
        let plan = plan(addon, "4.7.5").unwrap();
        (plan.record)(&Packed {
            layer: "sha256:00".to_owned(),
            layer_len: 1,
            inventory: layer::inventory(&plan.contents).unwrap(),
        })
    }

    #[test]
    fn ponytail_is_a_local_plugin_kai_names_around_its_pinned_skill() {
        let dir = PONYTAIL.dir();
        assert_eq!(
            PONYTAIL.files[0].path,
            format!("{dir}/{}", PONYTAIL.files[0].source)
        );
        let [manifest] = PONYTAIL.written else {
            panic!("one file kai writes: {:?}", PONYTAIL.written);
        };
        assert_eq!(manifest.path, format!("{dir}/.claude-plugin/plugin.json"));
        let manifest: Value = serde_json::from_slice(manifest.bytes).unwrap();
        let fields: Vec<&str> = manifest
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(fields, ["description", "name", "version"]);
        assert_eq!(manifest["name"], PONYTAIL.name);
    }

    #[test]
    fn ponytail_is_pinned_to_one_commit_one_skill_and_its_licence() {
        check(&PONYTAIL).unwrap();
        assert_eq!(named("ponytail").unwrap().commit, PONYTAIL.commit);
        assert!(named("caveman").is_none());
        let owner_repo = PONYTAIL
            .repository
            .strip_prefix("https://github.com/")
            .unwrap();
        for file in PONYTAIL.files {
            assert_eq!(
                file.artefact.url,
                format!(
                    "https://raw.githubusercontent.com/{owner_repo}/{}/{}",
                    PONYTAIL.commit, file.source
                )
            );
            assert_eq!(file.artefact.sha256.len(), 64);
            assert!(file.artefact.file.contains(PONYTAIL.commit));
        }
        let skill = format!("{}/skills/ponytail/SKILL.md", PONYTAIL.dir());
        assert!(
            PONYTAIL
                .files
                .iter()
                .any(|file| file.path == skill && file.source == "skills/ponytail/SKILL.md")
        );
        assert!(
            PONYTAIL
                .files
                .iter()
                .any(|file| file.path == PONYTAIL.licence_file && file.source == "LICENSE")
        );
        assert_eq!(PONYTAIL.files.len(), 2);
    }

    #[test]
    fn a_pin_kai_cannot_fetch_is_refused() {
        fn with(files: Vec<AddonFile>) -> Addon {
            Addon {
                files: Box::leak(files.into_boxed_slice()),
                ..PONYTAIL
            }
        }
        let skill = PONYTAIL.files[0];
        let licence = PONYTAIL.files[1];
        let moved = |path| AddonFile { path, ..skill };
        let mut program = skill;
        program.artefact.executable = true;
        let mut member = skill;
        member.artefact.member = Some("package/SKILL.md");
        let bad = [
            Addon {
                name: "pony/tail",
                ..PONYTAIL
            },
            Addon {
                name: "Ponytail",
                ..PONYTAIL
            },
            Addon {
                commit: "96608e0",
                ..PONYTAIL
            },
            Addon {
                commit: "main",
                ..PONYTAIL
            },
            Addon {
                licence: "",
                ..PONYTAIL
            },
            Addon {
                licence_file: "/opt/kai/addons/ponytail/COPYING",
                ..PONYTAIL
            },
            with(vec![]),
            with(vec![skill, licence, skill]),
            with(vec![moved("/opt/kai/addons/other/SKILL.md"), licence]),
            with(vec![moved("/opt/kai/agents/ponytail/SKILL.md"), licence]),
            with(vec![moved("/opt/kai/addons/ponytail/../SKILL.md"), licence]),
            with(vec![moved("/opt/kai/addons/ponytail"), licence]),
            with(vec![program, licence]),
            with(vec![member, licence]),
        ];
        let written = |path| Addon {
            written: Box::leak(Box::new([Written { path, bytes: b"{}" }])),
            ..PONYTAIL
        };
        let bad_written = [
            written("/opt/kai/addons/other/plugin.json"),
            written("/opt/kai/addons/ponytail/.claude-plugin/../x"),
            written(PONYTAIL.files[0].path),
        ];
        for addon in bad.into_iter().chain(bad_written) {
            assert!(check(&addon).is_err(), "{addon:?}");
        }
    }

    #[test]
    fn the_key_is_the_identity_as_canonical_json_and_changes_with_it() {
        let base = plan(&PONYTAIL, "4.7.5").unwrap().key;
        assert_eq!(base, plan(&PONYTAIL, "4.7.5").unwrap().key);
        let mut file = PONYTAIL.files[0];
        file.artefact.sha256 = "0000";
        let files: &'static [AddonFile] = Box::leak(Box::new([file, PONYTAIL.files[1]]));
        for (what, key) in [
            ("mksquashfs", plan(&PONYTAIL, "4.7.6").unwrap().key),
            (
                "commit",
                plan(
                    &Addon {
                        commit: "0000000000000000000000000000000000000000",
                        ..PONYTAIL
                    },
                    "4.7.5",
                )
                .unwrap()
                .key,
            ),
            (
                "licence",
                plan(
                    &Addon {
                        licence: "Apache-2.0",
                        ..PONYTAIL
                    },
                    "4.7.5",
                )
                .unwrap()
                .key,
            ),
            (
                "a file's hash",
                plan(&Addon { files, ..PONYTAIL }, "4.7.5").unwrap().key,
            ),
            (
                "the manifest",
                plan(
                    &Addon {
                        written: &[Written {
                            path: "/opt/kai/addons/ponytail/.claude-plugin/plugin.json",
                            bytes: b"{\"name\":\"ponytail\"}\n",
                        }],
                        ..PONYTAIL
                    },
                    "4.7.5",
                )
                .unwrap()
                .key,
            ),
        ] {
            assert_ne!(key, base, "{what}");
        }
        // The text the key hashes: keys sorted, no white space.
        let info = record(&PONYTAIL);
        let identity = json!({
            "commit": info.commit, "dir": info.dir, "files": info.files,
            "format": info.format, "licence": info.licence, "licence_file": info.licence_file,
            "mksquashfs": info.mksquashfs, "name": info.name, "packing": info.packing,
            "repository": info.repository, "written": info.written,
        })
        .to_string();
        assert!(identity.starts_with(r#"{"commit":"96608e07"#), "{identity}");
        assert!(!identity.contains(": ") && !identity.contains('\n'));
        assert_eq!(
            info.key,
            format!("sha256:{}", sha256_bytes(identity.as_bytes()))
        );
    }

    #[test]
    fn a_record_of_another_identity_is_named_by_the_field_that_differs() {
        let plan = plan(&PONYTAIL, "4.7.5").unwrap();
        let info = record(&PONYTAIL);
        assert_eq!((plan.mismatch)(&info), None);
        // The layer's hash and length are checked against the layer.
        let other_bytes = AddonInfo {
            layer: "sha256:11".to_owned(),
            layer_len: 2,
            ..info.clone()
        };
        assert_eq!((plan.mismatch)(&other_bytes), None);
        for (field, changed) in [
            (
                "`commit`",
                AddonInfo {
                    commit: "0".repeat(40),
                    ..info.clone()
                },
            ),
            (
                "`inventory`",
                AddonInfo {
                    inventory: "sha256:22".to_owned(),
                    ..info.clone()
                },
            ),
            (
                "`dir`",
                AddonInfo {
                    dir: "/opt/kai/addons/other".to_owned(),
                    ..info.clone()
                },
            ),
        ] {
            let why = (plan.mismatch)(&changed).unwrap();
            assert!(why.contains(field), "{why}");
        }
    }

    #[test]
    fn the_inventory_names_what_the_layer_holds_not_its_bytes() {
        let plan = plan(&PONYTAIL, "4.7.5").unwrap();
        let inventory = layer::inventory(&plan.contents).unwrap();
        assert!(inventory.starts_with("sha256:") && inventory.len() == 71);
        // Neither the packing nor the source of the bytes is in it.
        assert_eq!(
            inventory,
            layer::inventory(&super::plan(&PONYTAIL, "4.7.6").unwrap().contents).unwrap()
        );
        let mut moved = plan.contents;
        moved[0].path = "/opt/kai/addons/ponytail/.claude/skills/ponytail/SKILL.md".to_owned();
        assert_ne!(layer::inventory(&moved).unwrap(), inventory);
        moved[0].mode = 0o600;
        assert!(layer::inventory(&moved).is_err());
    }

    /// An invented add-on: a skill, its licence and a manifest kai writes.
    const INVENTED: Addon = Addon {
        name: "invented",
        repository: "https://example.invalid/invented",
        commit: "0123456789abcdef0123456789abcdef01234567",
        licence: "LicenseRef-invented",
        licence_file: "/opt/kai/addons/invented/LICENSE",
        files: &[
            AddonFile {
                source: "skills/invented/SKILL.md",
                path: "/opt/kai/addons/invented/skills/invented/SKILL.md",
                artefact: vm::Artefact {
                    name: "An invented skill",
                    file: "invented-SKILL.md",
                    url: "https://example.invalid/invented/0123456789abcdef0123456789abcdef01234567/skills/invented/SKILL.md",
                    download_sha256: "5555555555555555555555555555555555555555555555555555555555555555",
                    member: None,
                    sha256: "5555555555555555555555555555555555555555555555555555555555555555",
                    executable: false,
                },
                length: 120,
            },
            AddonFile {
                source: "LICENSE",
                path: "/opt/kai/addons/invented/LICENSE",
                artefact: vm::Artefact {
                    name: "An invented licence",
                    file: "invented-LICENSE",
                    url: "https://example.invalid/invented/0123456789abcdef0123456789abcdef01234567/LICENSE",
                    download_sha256: "6666666666666666666666666666666666666666666666666666666666666666",
                    member: None,
                    sha256: "6666666666666666666666666666666666666666666666666666666666666666",
                    executable: false,
                },
                length: 40,
            },
        ],
        written: &[Written {
            path: "/opt/kai/addons/invented/.claude-plugin/plugin.json",
            bytes: b"{\"name\":\"invented\"}\n",
        }],
    };

    #[test]
    fn kai_writes_and_reads_back_the_documented_example() {
        check(&INVENTED).unwrap();
        let plan = plan(&INVENTED, "4.7.5").unwrap();
        let written = (plan.record)(&Packed {
            layer: format!("sha256:{}", "7".repeat(64)),
            layer_len: 4096,
            inventory: layer::inventory(&plan.contents).unwrap(),
        });
        let text = include_str!("../../../contracts/kai-addon-layer-1.example.json");
        assert_eq!(
            serde_json::to_value(&written).unwrap(),
            serde_json::from_str::<Value>(text).unwrap(),
            "{}",
            serde_json::to_string_pretty(&written).unwrap()
        );
        // Read back as the cache reads an entry's JSON: it names the key.
        let example: AddonInfo = serde_json::from_str(text).unwrap();
        assert_eq!((plan.mismatch)(&example), None);
    }
}
