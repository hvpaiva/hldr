//! Content shared by the unit tests.

use std::fs;
use std::path::Path;

pub const SITE: &str = "\
kind: Site
title: example.test
theme: nord
banner:
  alt: EX
  art: |-
    ##
     #
descriptions:
  projects: Work.
  themes: Palettes.
blog:
  enabled: false
";

pub const THEME: &str = "\
kind: Theme
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

/// Writes `site.yaml` and the theme it names under `dir`.
pub fn write_site(dir: &Path) {
    fs::write(dir.join("site.yaml"), SITE).unwrap();
    fs::create_dir_all(dir.join("themes")).unwrap();
    fs::write(dir.join("themes/nord.yaml"), THEME).unwrap();
}
