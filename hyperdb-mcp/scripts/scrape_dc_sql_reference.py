"""
Scrape the Salesforce Data Cloud SQL reference into a single markdown file.

PURPOSE
-------
The SQL dialect cheat-sheet embedded in `server.rs` (the `sql_dialect` string
in `HyperMcpServer::get_info`) was produced by running this script and then
manually distilling the output into the bullet-point summary in the source.

Run this script periodically (e.g. when a new Data Cloud release ships) to
check whether the reference has grown new sections, changed type lists, or
added/removed functions.  Diff the output against the previous scrape and
update `server.rs` accordingly.

USAGE
-----
Install the one dependency (Playwright + a Chromium headless shell) once:

    uv run --with playwright python3 -m playwright install chromium

Then scrape:

    uv run --with playwright python3 scripts/scrape_dc_sql_reference.py

Output is written to:  scripts/dc_sql_reference.md  (~40 KB, ~12 pages)

If `uv` is not available, a plain virtualenv works too:

    python3 -m venv .venv
    source .venv/bin/activate
    pip install playwright
    python3 -m playwright install chromium
    python3 scripts/scrape_dc_sql_reference.py

WHAT TO DO WITH THE OUTPUT
--------------------------
1. Open scripts/dc_sql_reference.md and skim for new sections.
2. Compare against the `sql_dialect` string in src/server.rs.
3. Edit src/server.rs to reflect any additions or removals.
4. Run `cargo fmt` and `cargo clippy` before committing.

The script does NOT automatically update server.rs — the distillation step
is intentional: the raw reference is ~40 KB; the instructions field should
stay concise enough to fit in a typical LLM context window.
"""

import asyncio
import re
import sys
from pathlib import Path
from urllib.parse import urlparse

from playwright.async_api import async_playwright

# The index page — Playwright will collect all sidebar links from here.
INDEX_URL = "https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/data-cloud-sql-context.html"

# Only keep links that belong to this section.
SECTION_PREFIX = "https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/"

# Output file lives next to this script so it's easy to diff over time.
OUT_FILE = Path(__file__).parent / "dc_sql_reference.md"


async def collect_nav_links(page, url: str) -> list[str]:
    await page.goto(url, wait_until="networkidle", timeout=60_000)
    links = await page.eval_on_selector_all("a[href]", "els => els.map(e => e.href)")
    seen: set[str] = set()
    result: list[str] = []
    for link in links:
        # Strip fragment — we want unique pages, not same-page anchors.
        bare = link.split("#")[0]
        if bare.startswith(SECTION_PREFIX) and bare not in seen:
            seen.add(bare)
            result.append(bare)
    return result


async def scrape_page(page, url: str) -> str:
    try:
        await page.goto(url, wait_until="networkidle", timeout=60_000)
    except Exception as e:
        return f"\n\n---\n\n## {url}\n\n_Error loading page: {e}_\n\n"

    try:
        await page.wait_for_selector("article, main, [role='main']", timeout=15_000)
    except Exception:
        pass

    title = await page.title()
    title = re.sub(r"\s*[|\-–]\s*Salesforce.*$", "", title).strip()

    content = await page.eval_on_selector_all(
        "article *, main *",
        """els => {
            const blocks = [];
            for (const el of els) {
                if (el.closest('nav, aside, header, footer, .sidebar, .toc')) continue;
                const tag = el.tagName.toLowerCase();
                const text = el.innerText?.trim();
                if (!text) continue;
                if (['h1','h2','h3','h4'].includes(tag)) {
                    const level = '#'.repeat(parseInt(tag[1]) + 1);
                    blocks.push(level + ' ' + text);
                } else if (tag === 'pre' || tag === 'code') {
                    blocks.push('```\\n' + text + '\\n```');
                } else if (tag === 'li') {
                    blocks.push('- ' + text);
                } else if (['p', 'td', 'th'].includes(tag)) {
                    blocks.push(text);
                }
            }
            const deduped = [];
            for (const b of blocks) {
                if (deduped[deduped.length - 1] !== b) deduped.push(b);
            }
            return deduped.join('\\n\\n');
        }"""
    )

    if not content:
        content = await page.inner_text("body")

    slug = url.replace(SECTION_PREFIX, "").replace(".html", "")
    return f"\n\n---\n\n# {title or slug}\n\n_URL: {url}_\n\n{content}\n"


async def main() -> None:
    async with async_playwright() as pw:
        browser = await pw.chromium.launch(headless=True)
        page = await browser.new_page()

        print("Collecting nav links …", file=sys.stderr)
        links = await collect_nav_links(page, INDEX_URL)
        if not links:
            links = [INDEX_URL]
        print(f"Found {len(links)} page(s)", file=sys.stderr)

        parts = [
            f"# Salesforce Data Cloud SQL Reference\n\n"
            f"_Scraped from: {INDEX_URL}_\n\n"
            f"_To refresh: `uv run --with playwright python3 scripts/scrape_dc_sql_reference.py`_\n"
        ]
        for i, url in enumerate(links, 1):
            print(f"  [{i}/{len(links)}] {url}", file=sys.stderr)
            parts.append(await scrape_page(page, url))

        await browser.close()

    OUT_FILE.write_text("\n".join(parts), encoding="utf-8")
    size_kb = OUT_FILE.stat().st_size // 1024
    print(f"\nWrote {OUT_FILE} ({size_kb} KB, {len(links)} page(s))", file=sys.stderr)
    print(f"Next step: diff against previous version and update the sql_dialect string in src/server.rs")


if __name__ == "__main__":
    asyncio.run(main())
