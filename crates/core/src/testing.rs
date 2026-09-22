//! Content shared by the unit tests: a small tree in the layout of
//! hldr-content.

use std::fs;
use std::path::Path;

pub const SITE: &str = "\
kind: Site
spec:
  title: example.test
  theme: nord
  values:
    banner:
      alt: EX
      art: |-
        ##
         #
    about: Design to platform.
";

pub const PROFILE: &str = "\
kind: Profile
spec:
  name: Highlander Paiva
  headline: senior platform engineer
  bio: writes software.
  email: contact@hvpaiva.dev
  links:
    github: https://github.com/hvpaiva
";

pub const NAV: &str = "\
kind: Nav
spec:
  sections:
    - name: files
      title: \"{{ site.title }}\"
      items:
        - page: index
        - page: about
        - collection: projects
          open_max: 8
    - name: commands
      items:
        - command: colorscheme
        - command: help
    - name: elsewhere
      items:
        - link: profile.links.github
        - link: profile.email
          label: mail
  statusline:
    count: projects
";

pub const THEME: &str = "\
kind: Theme
metadata:
  name: nord
spec:
  title: Nord
  dark: true
  colors:
    bg: \"#2E3440\"
    fg: \"#d8dee9\"
    fg_dim: \"#e5e9f0\"
    fg_muted: \"#4c566a\"
    accent: \"#88c0d0\"
    accent2: \"#81a1c1\"
    highlight: \"#ebcb8b\"
    special: \"#b48ead\"
    teal: \"#8fbcbb\"
    error: \"#bf616a\"
    border: \"#3b4252\"
";

pub const PROJECT_KIND: &str = "\
kind: PageKind
metadata:
  name: project
spec:
  names: {kind: Project, singular: project, plural: projects, short_names: [p]}
  fields:
    tagline: {type: string, required: true}
    status: {type: enum, values: [active, wip, archived], ok: [active], required: true}
    highlight: {type: int}
    tags: {type: list}
    links: {type: links, keys: [repo, docs, demo]}
    github: {type: string}
  summary: tagline
  footer: |

    {{ clone page.links.repo }}
    {{ link /projects \"← projects/\" }}
  json_ld:
    type: SoftwareSourceCode
    map: {description: tagline, codeRepository: links.repo, programmingLanguage: tags}
";

pub const PROJECTS: &str = "\
kind: Collection
metadata:
  name: projects
spec:
  kind: Project
  description: Work.
  order: highlight
  badge: status
  columns:
    - {field: status, width: 9}
    - {field: tags, header: stack}
";

pub const ATLAS: &str = "\
kind: Project
metadata:
  name: atlas
  title: Atlas
  tagline: CubeSat ADCS in Rust
  status: active
  highlight: 1
  tags: [rust, aerospace]
  links:
    repo: https://github.com/example/atlas
  github: example/atlas
spec:
  frontmatter: open
  content:
    file: atlas.md
";

pub const ATLAS_MD: &str = "## What it is\n\nContext and **decisions**.\n";

pub const INDEX: &str = "\
kind: Page
metadata:
  name: index
  title: \"{{ profile.name }}\"
  description: \"{{ profile.bio }}\"
spec:
  buffer: README.md
  standalone_title: true
  json_ld: person
  content:
    file: index.md
";

pub const INDEX_MD: &str = "\
{{ banner site.values.banner }}

# {{ profile.name }}

## {{ link /projects Projects }}
{{ collection projects where=highlight limit=3 }}

## About
{{ site.values.about }} More in {{ link /about about.md }}.
";

pub const ABOUT: &str = "\
kind: Page
metadata:
  name: about
  title: about
  description: \"{{ profile.headline }}\"
spec:
  json_ld: person
  content:
    file: about.md
";

pub const ABOUT_MD: &str = "# about\n\n{{ site.values.about }}\n\n## Contact\n{{ links }}\n";

pub const NOT_FOUND: &str = "\
kind: Page
metadata:
  name: not-found
  title: \"404\"
spec:
  content:
    inline: |
      {{ link / README.md }}      start here
";

pub const THEME_PAGE: &str = "\
kind: Page
metadata:
  name: theme
  title: theme
  description: Palettes.
spec:
  buffer: \"[colorscheme]\"
  style: colorscheme
";

/// A tree that validates: every file of the layout, once.
pub const TREE: [(&str, &str); 14] = [
    ("site.yaml", SITE),
    ("profile.yaml", PROFILE),
    ("nav.yaml", NAV),
    ("themes/nord.yaml", THEME),
    ("kinds/project.yaml", PROJECT_KIND),
    ("collections/projects.yaml", PROJECTS),
    ("projects/atlas.yaml", ATLAS),
    ("projects/atlas.md", ATLAS_MD),
    ("pages/index.yaml", INDEX),
    ("pages/index.md", INDEX_MD),
    ("pages/about.yaml", ABOUT),
    ("pages/about.md", ABOUT_MD),
    ("pages/not-found.yaml", NOT_FOUND),
    ("pages/theme.yaml", THEME_PAGE),
];

/// Writes `files` under `dir`, creating directories as needed.
pub fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
}
