#!/usr/bin/env python3
"""Download the most popular open-source TradingView scripts into a local, untracked cache.

    python3 tools/pine-corpus/fetch.py [--indicators 20] [--libraries 10]

Sources land in `.pine-corpus/<kind>/<slug>.pine` (gitignored). Only metadata is written to
`DegenViewTests/PineCorpus/manifest.json`, which is committed. See README.md for the rules.
"""

import argparse
import datetime
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CACHE_DIR = os.path.join(ROOT, ".pine-corpus")
MANIFEST = os.path.join(ROOT, "DegenViewTests", "PineCorpus", "manifest.json")

USER_AGENT = "Mozilla/5.0 (DegenView pine-corpus fetcher; local compatibility testing)"
REQUEST_DELAY = 1.0
MAX_PAGES = 20
SUPPORTED_VERSION = 6

LISTING_URL = "https://www.tradingview.com/scripts/{page}?script_type={kind}"
SOURCE_URL = "https://pine-facade.tradingview.com/pine-facade/get/{pub}/last"

# A listing embeds one JSON object per script; `chart_url` precedes `script_id_part`.
ENTRY = re.compile(
    r'"chart_url":"(?P<url>[^"]+)".*?"script_id_part":"(?P<pub>PUB;[0-9a-f]+)"', re.S
)
# The source can start with several comment lines; the licence notice is one of them.
LICENCE_LINE = re.compile(r"subject to the terms of the (?P<name>.+?) at ", re.I)
VERSION_LINE = re.compile(r"^\s*//\s*@version\s*=\s*(\d+)", re.M)
SLUG = re.compile(r"/script/(?P<id>[A-Za-z0-9]+)(?:-(?P<slug>[^/]+))?/")


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read().decode("utf-8")
    time.sleep(REQUEST_DELAY)
    return body


def listing(kind, page):
    """Scripts on one page of the popularity-ordered listing, as (url, pub id) pairs."""
    path = "" if page == 1 else "page-%d/" % page
    html = get(LISTING_URL.format(page=path, kind=kind))
    seen = {}
    for match in ENTRY.finditer(html):
        seen.setdefault(match.group("pub"), match.group("url"))
    return [(url, pub) for pub, url in seen.items()]


def licence_of(source):
    for line in source.splitlines()[:12]:
        match = LICENCE_LINE.search(line)
        if match:
            name = match.group("name")
            return "MPL-2.0" if "Mozilla Public License 2.0" in name else name
    return "unknown"


def describe(url, pub, kind):
    """Fetch one script. Returns (manifest entry, source or None)."""
    ids = SLUG.search(url)
    if ids is None:
        raise ValueError("unrecognised script URL: " + url)
    info = json.loads(get(SOURCE_URL.format(pub=urllib.parse.quote(pub, safe=""))))
    source = info.get("source") or ""
    version = VERSION_LINE.search(source)
    entry = {
        "slug": ids.group("id"),
        "name": info.get("scriptName", ""),
        "kind": kind,
        "url": url,
        "tradingViewID": pub,
        "licence": licence_of(source),
        "pineVersion": int(version.group(1)) if version else None,
        "fetchedAt": datetime.date.today().isoformat(),
    }
    if info.get("scriptAccess") != "open_no_auth":
        entry["status"] = "skipped: not open source"
    elif entry["pineVersion"] != SUPPORTED_VERSION:
        entry["status"] = "skipped: pine v%s" % entry["pineVersion"]
    else:
        entry["status"] = "fetched"
        return entry, source
    return entry, None


def collect(kind, singular, wanted):
    entries = []
    taken = 0
    for page in range(1, MAX_PAGES + 1):
        for url, pub in listing(kind, page):
            if taken >= wanted:
                return entries
            try:
                entry, source = describe(url, pub, singular)
            except (urllib.error.URLError, ValueError, json.JSONDecodeError) as error:
                print("  ! %s: %s" % (url, error), file=sys.stderr)
                continue
            entries.append(entry)
            if source is not None:
                directory = os.path.join(CACHE_DIR, singular)
                os.makedirs(directory, exist_ok=True)
                with open(os.path.join(directory, entry["slug"] + ".pine"), "w") as handle:
                    handle.write(source)
                taken += 1
            print("  %-10s %-45s %s" % (entry["slug"], entry["name"][:45], entry["status"]))
    if taken < wanted:
        raise SystemExit("only %d of %d %s found in %d pages" % (taken, wanted, kind, MAX_PAGES))
    return entries


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--indicators", type=int, default=20)
    parser.add_argument("--libraries", type=int, default=10)
    args = parser.parse_args()

    entries = []
    for kind, singular, wanted in (
        ("indicators", "indicator", args.indicators),
        ("libraries", "library", args.libraries),
    ):
        print("%s (top %d, pine v%d only)" % (kind, wanted, SUPPORTED_VERSION))
        entries += collect(kind, singular, wanted)

    os.makedirs(os.path.dirname(MANIFEST), exist_ok=True)
    with open(MANIFEST, "w") as handle:
        json.dump(entries, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    print("wrote %s (%d entries)" % (os.path.relpath(MANIFEST, ROOT), len(entries)))


if __name__ == "__main__":
    main()
