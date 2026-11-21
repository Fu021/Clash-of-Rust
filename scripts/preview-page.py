"""Capture a Markdown document or web page as a PNG."""

import argparse
from html import escape
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit


PREVIEW_ORIGIN = "http://preview.invalid"
MARKDOWN_STYLE = """
:root { color-scheme: light dark; }
body {
    margin: 0;
    font: 16px/1.65 system-ui, "Noto Sans CJK SC", "Microsoft YaHei", sans-serif;
    color: light-dark(#202632, #e6e9f0);
    background: light-dark(#f4f6f9, #171a21);
}
main {
    box-sizing: border-box;
    width: min(100%, 1040px);
    min-height: 100vh;
    margin: auto;
    padding: 28px clamp(20px, 5vw, 64px) 48px;
    background: light-dark(#fff, #21252e);
    overflow-wrap: anywhere;
}
h1, h2, h3 { line-height: 1.3; margin: 1.5em 0 .6em; }
h1:first-child { margin-top: .4em; }
h1, h2 { padding-bottom: .32em; border-bottom: 1px solid light-dark(#d7e0ea, #424a59); }
a { color: light-dark(#1455a0, #8fc5ff); }
img { max-width: 100%; height: auto; }
code { padding: .1em .32em; border-radius: 4px; background: light-dark(#eff2f6, #303645); }
pre { padding: 14px 18px; border-radius: 6px; background: light-dark(#eff2f6, #303645); }
pre code { padding: 0; white-space: pre-wrap; }
blockquote { margin-inline: 0; padding-inline: 1em; border-left: 4px solid #8294aa; }
table { display: block; width: max-content; max-width: 100%; overflow-x: auto; border-collapse: collapse; }
th, td { padding: .45em .75em; border: 1px solid light-dark(#d7e0ea, #424a59); }
th { background: light-dark(#eff2f6, #303645); }
li + li { margin-top: .25em; }
"""
MARKDOWN_CSP = "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; base-uri 'none'"


def markdown_document(source):
    from markdown_it import MarkdownIt

    markdown = MarkdownIt("commonmark").enable(["table", "strikethrough"])
    title = escape(source.stem)
    content = markdown.render(source.read_text(encoding="utf-8"))
    return (
        '<!doctype html><html lang="zh-CN"><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>{title}</title><style>{MARKDOWN_STYLE}</style><main>{content}</main></html>"
    )


def serve_local(route, root, source, markdown):
    url = urlsplit(route.request.url)
    if f"{url.scheme}://{url.netloc}" != PREVIEW_ORIGIN:
        route.abort()
        return

    path = (root / unquote(url.path).lstrip("/")).resolve()
    if not path.is_relative_to(root):
        route.fulfill(status=403, body="")
    elif path == source and markdown is not None:
        route.fulfill(
            body=markdown, content_type="text/html; charset=utf-8",
            headers={"Content-Security-Policy": MARKDOWN_CSP},
        )
    elif path.is_file():
        route.fulfill(path=path)
    else:
        route.fulfill(status=404, body="")


def capture(args, source, root, url):
    from playwright.sync_api import sync_playwright

    markdown = markdown_document(source) if source and source.suffix.lower() == ".md" else None
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            context = browser.new_context(
                viewport={"width": args.width, "height": args.height},
                color_scheme=args.theme,
                reduced_motion="reduce",
                java_script_enabled=markdown is None,
                service_workers="block",
            )
            if root is not None:
                context.route("**/*", lambda route: serve_local(route, root, source, markdown))
            page = context.new_page()
            response = page.goto(url, wait_until="load", timeout=30000)
            if response is not None and not response.ok:
                raise ValueError(f"HTTP {response.status}: {url}")
            if args.full_page:
                page.locator('img[loading="lazy"]').evaluate_all(
                    "(images) => images.forEach(image => { image.loading = 'eager'; })"
                )
                page.wait_for_function("Array.from(document.images).every(image => image.complete)")
            return page.screenshot(full_page=args.full_page, animations="disabled")
        finally:
            browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", help="a Markdown file, HTML file, or HTTP(S) URL")
    parser.add_argument("--output", required=True, type=Path, help="destination PNG file")
    parser.add_argument("--root", type=Path, help="local asset root (defaults to the input file's directory)")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=900)
    parser.add_argument("--theme", choices=("light", "dark"), default="light", help="browser color preference")
    length = parser.add_mutually_exclusive_group()
    length.add_argument("--full-page", action="store_true", default=None, help="capture the entire scrollable page")
    length.add_argument("--viewport-only", action="store_false", dest="full_page", help="capture only the viewport")
    args = parser.parse_args()

    if min(args.width, args.height) < 1:
        parser.error("width and height must be positive")
    if args.output.suffix.lower() != ".png" or args.output.exists():
        parser.error("--output must be a new .png file")

    parsed_url = urlsplit(args.source)
    if parsed_url.scheme in ("http", "https") and parsed_url.netloc:
        if args.root:
            parser.error("--root only applies to local files")
        source, root, url = None, None, args.source
    else:
        source = Path(args.source).resolve()
        if not source.is_file() or source.suffix.lower() not in (".md", ".html", ".htm"):
            parser.error("source must be an existing .md or .html file, or an HTTP(S) URL")
        root = args.root.resolve() if args.root else source.parent
        if not root.is_dir() or not source.is_relative_to(root):
            parser.error("--root must contain the input file")
        url = f"{PREVIEW_ORIGIN}/{quote(source.relative_to(root).as_posix())}"

    if args.full_page is None:
        args.full_page = source is None or source.suffix.lower() != ".md"

    try:
        image = capture(args, source, root, url)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("xb") as output:
            output.write(image)
    except (ImportError, OSError, ValueError) as error:
        parser.exit(1, f"preview-page: {error}\n")
    print(args.output.resolve())


if __name__ == "__main__":
    main()
