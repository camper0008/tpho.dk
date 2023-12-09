use std::cmp::Ordering;
use std::env;
use std::fs::{self, ReadDir};
use std::path::PathBuf;

use anyhow::Context;
use itertools::Itertools;

enum Leaf<T> {
    Dir(Vec<(String, Leaf<T>)>),
    File(T),
}

fn file_tree(file_name: String, dir: ReadDir) -> anyhow::Result<(String, Leaf<Vec<u8>>)> {
    let entries = dir
        .map(|entry| {
            let entry = entry?;
            let file_name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "expected file {:?} to have valid utf-8 filename",
                        entry.path()
                    )
                })?
                .to_string();

            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                file_tree(file_name, fs::read_dir(entry.path())?)
            } else {
                Ok((file_name, Leaf::File(fs::read(entry.path())?)))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok((file_name, Leaf::Dir(entries)))
}

fn breadcrumbs_html(breadcrumbs: &Vec<String>) -> String {
    let mut result = Vec::new();
    let mut path = String::new();
    let mut idx = 0;
    loop {
        if idx == 0 {
            result.push(format!(r#"<a href="/">{}</a>"#, "root"));
            idx += 1;
            if idx == breadcrumbs.len() {
                break;
            } else {
                continue;
            }
        }
        let name = &breadcrumbs[idx];
        if breadcrumbs.len() - 1 == idx
            || breadcrumbs.len() - 2 == idx && breadcrumbs[idx + 1].starts_with("README")
        {
            result.push(format!(r#"<span>{name}</span>"#));
            break;
        } else {
            path += &format!("/{name}");
            result.push(format!(r#"<a href="{path}">{name}</a>"#))
        }
        idx += 1;
    }
    if result.len() == 1 {
        String::new()
    } else {
        result.join(" / ")
    }
}

fn is_text_file<T: AsRef<str>>(name: T) -> bool {
    name.as_ref().ends_with(".txt") || name.as_ref().ends_with(".md")
}

fn write_text_file(
    mut path: PathBuf,
    content: Vec<u8>,
    formatted_breadcrumbs: String,
    name: String,
) -> anyhow::Result<()> {
    let template = include_str!("templates/root.html");
    let template = template.replace("{{breadcrumbs}}", &formatted_breadcrumbs);

    let content = std::str::from_utf8(&content)
        .with_context(|| format!("file '{name}' contains invalid utf-8"))?;
    let content = if name.ends_with(".md") {
        markdown::to_html(content)
    } else {
        format!("<pre class=\"text-file\">{content}</pre>")
    };

    let template = template.replace("{{content}}", &content);
    if path.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("")) == "README" {
        path.set_file_name("index.html");
    } else {
        path.set_extension("html");
    };

    fs::write(&path, template).with_context(|| format!("unable to write to {path:?}"))?;
    Ok(())
}

fn write_dir_index(
    path: PathBuf,
    breadcrumbs: &Vec<String>,
    name: String,
    files: Vec<(String, Leaf<Vec<u8>>)>,
) -> anyhow::Result<()> {
    fs::create_dir(&path).with_context(|| format!("unable to create dir at {path:?}"))?;
    let files_html = files
        .iter()
        .sorted_by(
            |(name_a, leaf_a), (name_b, leaf_b)| match (leaf_a, leaf_b) {
                (Leaf::Dir(_), Leaf::Dir(_)) | (Leaf::File(_), Leaf::File(_)) => name_a.cmp(name_b),
                (Leaf::Dir(_), Leaf::File(_)) => Ordering::Less,
                (Leaf::File(_), Leaf::Dir(_)) => Ordering::Greater,
            },
        )
        .map(|(name, leaf)| {
            let mut path = breadcrumbs.clone();
            path.remove(0);
            if let Some(stem) = name
                .strip_suffix(".md")
                .or_else(|| name.strip_suffix(".txt"))
            {
                if stem == "README" {
                    path.push("index.html".to_string())
                } else {
                    path.push(stem.to_string())
                }
            } else {
                path.push(name.to_owned());
            }
            let path = path.join("/");
            let class = match leaf {
                Leaf::Dir(_) => "directory-listing",
                Leaf::File(_) if is_text_file(name) => "text-file-listing",
                Leaf::File(_) => "file-listing",
            };
            format!(r#"<li class="{class}"><a href="/{path}">{name}</a></li>"#)
        })
        .collect::<String>();
    let template = include_str!("templates/directory_list.html");
    let content = template
        .replace("{{content}}", &files_html)
        .replace("{{name}}", &name);

    let content = include_str!("templates/root.html")
        .replace("{{breadcrumbs}}", &breadcrumbs_html(breadcrumbs))
        .replace("{{content}}", &content);
    {
        let mut path = path.clone();
        path.push("index.html");
        fs::write(path, content)?;
    }
    for leaf in files {
        build_html(leaf, path.clone(), breadcrumbs.clone())?;
    }

    Ok(())
}

fn build_html(
    leaf: (String, Leaf<Vec<u8>>),
    mut path: PathBuf,
    mut breadcrumbs: Vec<String>,
) -> anyhow::Result<()> {
    let (name, leaf) = leaf;
    if breadcrumbs.len() > 0 {
        path.push(&name);
    }
    breadcrumbs.push(name.clone());

    match leaf {
        Leaf::Dir(files) => write_dir_index(path, &breadcrumbs, name, files)?,
        Leaf::File(content) => {
            if is_text_file(&name) {
                write_text_file(path, content, breadcrumbs_html(&breadcrumbs), name)?;
            } else {
                fs::write(&path, content).with_context(|| format!("unable to write to {path:?}"))?
            }
        }
    }
    Ok(())
}

fn copy_dir(from: impl Into<PathBuf>, to: impl Into<PathBuf>) -> anyhow::Result<()> {
    let from = from.into();
    let to = to.into();
    for file in fs::read_dir(&from)? {
        let mut from = from.clone();
        let mut to = to.clone();

        let file = file?;
        let file_name = file.file_name();
        from.push(&file_name);
        to.push(&file_name);
        if file.file_type()?.is_dir() {
            fs::create_dir(&to)?;
            copy_dir(from, to)?;
        } else {
            fs::copy(from, to)?;
        };
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let build_dir = env::var("OUT_DIR").unwrap_or_else(|_| "build".into());
    let _ = fs::remove_dir_all(&build_dir);
    let root_name = env::var("ROOT_TITLE").unwrap_or_else(|_| "root".into());
    let tree = file_tree(root_name, fs::read_dir("content")?)?;
    build_html(tree, build_dir.clone().into(), Vec::new())?;
    copy_dir("public", &build_dir)?;
    Ok(())
}
