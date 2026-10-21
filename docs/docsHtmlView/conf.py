# docs-html-view managed Sphinx config (template).
#
# Apply only to the conf.py that sphinx-quickstart just generated for a NEW site:
# copy this file over docs/docsHtmlView/conf.py and replace the __TOKENS__ below,
# then copy assets/guard.py to docs/docsHtmlView/guard.py. For an existing site,
# do not overwrite: show the user a manual merge plan first.
#
# The publication is driven by docs/toc.yml: run scripts/gen_prd.py before every
# build. The generated _meta.json and _manifest.json are the only source of the
# title/version and the exact include list; an unlisted document is never built.
import json
import os
import sys

sys.path.insert(0, os.path.abspath(os.path.dirname(__file__)))

_CONFIG_DIR = os.path.dirname(os.path.abspath(__file__))


def _load_generated(name):
    path = os.path.join(_CONFIG_DIR, name)
    if not os.path.isfile(path):
        raise RuntimeError(
            "docs-html-view: missing %s; run scripts/gen_prd.py before sphinx-build" % path
        )
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


_meta = _load_generated("_meta.json")
_manifest = _load_generated("_manifest.json")
_manifest_docs = _manifest.get("documents") if isinstance(_manifest, dict) else _manifest
if not isinstance(_manifest_docs, list) or not _manifest_docs:
    raise RuntimeError("docs-html-view: _manifest.json has no documents; re-run scripts/gen_prd.py")

project = _meta.get("title") or "TodoList"
author = _meta.get("author") or "TodoList"
release = _meta.get("version") or "1.0"

extensions = ["myst_parser", "guard"]
html_theme = "sphinx_rtd_theme"
html_theme_options = {}

source_suffix = {".md": "markdown", ".rst": "restructuredtext"}

# Cover + section wrappers + generated glossary, plus the exact TOC documents.
# Deliberately not a docsHtmlView/** wildcard: unlisted sources are excluded.
root_doc = "docsHtmlView/index"
include_patterns = [
    "docsHtmlView/index.md",
    "docsHtmlView/nav/**",
    "docsHtmlView/glossary.md",
] + list(_manifest_docs)

exclude_patterns = [
    "docsHtmlView/build/**",
    "docsHtmlView/.venv*/**",
    ".venv-docs/**",
    "**/.git/**",
]

# Read by guard.py; must match the publication roots in docs/toc.yml.
docs_html_view_allowed_roots = ["product", "current", "adr"]

myst_enable_extensions = ["colon_fence", "deflist", "fieldlist"]
myst_heading_anchors = 3
