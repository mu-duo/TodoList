"""docs-html-view guard: a best-effort parse/render guard for trusted project docs.

This is a trusted-document renderer, not a universal sandbox. It blocks
file-insertion directives, refuses active/raw HTML (allowing only HTML comments),
and checks local image/download references against the approved roots at
``doctree-read`` (before Sphinx's asset collectors copy anything). Remote
images/downloads are never fetched. The project's own ``conf.py`` remains
executable Python.
"""
from __future__ import annotations

import os
import re
from urllib.parse import urlparse

from docutils import nodes
from docutils.parsers.rst import Directive, directives
from sphinx import addnodes
from sphinx.errors import SphinxError
from sphinx.util import logging

logger = logging.getLogger(__name__)

BLOCKED = ("include", "literalinclude", "raw")
# A raw node is allowed only when it is entirely HTML comment(s) and whitespace.
# This permits the harmless `<!-- ... -->` comments used in the PRD templates while
# still refusing any active markup (no comment+script smuggling).
_COMMENT_ONLY = re.compile(r"\s*(?:<!--.*?-->\s*)*", re.DOTALL)


class _Blocked(Directive):
    has_content = True
    optional_arguments = 1
    final_argument_whitespace = True

    def run(self):
        raise SphinxError("docs-html-view: '%s' directive is not allowed" % self.name)


def _block_directives(app):
    # Registered after extensions load; docutils lowercases names, so this also
    # catches INCLUDE and directives reached through eval-rst or colon fences.
    for name in BLOCKED:
        directives.register_directive(name, _Blocked)


def _validate_roots(app):
    """Every allowed root must be a simple relative name that resolves inside docs."""
    srcdir_real = os.path.realpath(app.srcdir)
    for name in app.config.docs_html_view_allowed_roots:
        if not isinstance(name, str) or not name or os.path.isabs(name) or "\\" in name:
            raise SphinxError("docs-html-view: invalid allowed root %r" % (name,))
        parts = name.split("/")
        if any(part in ("", ".", "..") for part in parts):
            raise SphinxError("docs-html-view: allowed root must be a simple relative name: %r" % (name,))
        root_real = os.path.realpath(os.path.join(app.srcdir, name))
        try:
            inside = os.path.commonpath([root_real, srcdir_real]) == srcdir_real
        except ValueError:
            inside = False
        if not inside:
            raise SphinxError("docs-html-view: allowed root escapes docs: %r" % (name,))


def _remote(uri: str) -> bool:
    return bool(urlparse(uri).scheme) or uri.startswith("//")


def _within(path: str, roots) -> bool:
    real = os.path.realpath(path)
    for root in roots:
        root_real = os.path.realpath(root)
        try:
            if os.path.commonpath([real, root_real]) == root_real:
                return True
        except ValueError:
            continue
    return False


def _guard_assets(app, doctree):
    docname = app.env.docname
    roots = [os.path.join(app.srcdir, r) for r in app.config.docs_html_view_allowed_roots]

    for node in doctree.findall(nodes.raw):
        if not _COMMENT_ONLY.fullmatch(node.astext() or ""):
            raise SphinxError("docs-html-view: active/raw HTML is not allowed in %s" % docname)

    def check(uri, kind):
        """True if usable locally, False if remote; raises when local-but-outside."""
        if not uri:
            return True
        if _remote(uri):
            return False
        clean = uri.split("#", 1)[0]
        try:
            _rel, abspath = app.env.relfn2path(clean, docname)
        except Exception:
            abspath = None
        if abspath is None or not _within(abspath, roots):
            raise SphinxError(
                "docs-html-view: %s outside approved roots in %s: %s" % (kind, docname, uri)
            )
        return True

    for node in doctree.findall(nodes.image):
        uri = node.get("uri", "")
        if check(uri, "image"):
            continue
        logger.warning("docs-html-view: remote image not fetched: %s", uri, location=docname)
        node.replace_self(nodes.emphasis("", "[remote image not fetched] " + uri))

    for node in doctree.findall(addnodes.download_reference):
        uri = node.get("reftarget") or node.get("refuri") or ""
        if check(uri, "download"):
            continue
        logger.warning("docs-html-view: remote download not fetched: %s", uri, location=docname)
        node.replace_self(nodes.emphasis("", "[remote download not fetched] " + uri))


def setup(app):
    app.add_config_value("docs_html_view_allowed_roots", [], "env")
    app.connect("builder-inited", _block_directives)
    app.connect("builder-inited", _validate_roots)
    # doctree-read (priority 100) so we run before Sphinx's asset collectors (500),
    # which copy downloads during read and register images for copy.
    app.connect("doctree-read", _guard_assets, priority=100)
    return {"parallel_read_safe": True, "parallel_write_safe": True}
