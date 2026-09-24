"""Third-party bundled-data notices for Lotus (CLAUDE.md rule 7).

    py tools/notices/notices.py generate [--manifest-path Cargo.toml]
    py tools/notices/notices.py check    [--manifest-path Cargo.toml] [--strict]
    py tools/notices/notices.py scan     [--manifest-path Cargo.toml]   (list what the scan finds)

`generate` writes THIRD_PARTY_NOTICES.txt at the repo root from tools/notices/data.json.
`check` fails if the dependency graph embeds a data file no manifest entry covers, if a
manifest version differs from Cargo.lock, if a licence is outside the rule, if an `ignored`
entry is unexplained (neither behind a feature that is off nor verifiably test-only) or no
longer matches anything, or if THIRD_PARTY_NOTICES.txt is stale. Our own embedded files are
listed under `first_party` (LicenseRef-Lotus-Proprietary), not ignored. Stdlib only.
"""
import fnmatch, glob, json, os, re, subprocess, sys, textwrap, tomllib, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DATA = os.path.join(HERE, 'data.json')
OUT = os.path.join(REPO, 'THIRD_PARTY_NOTICES.txt')

# CLAUDE.md rule 7: allowed only for data files bundled unmodified, never for code.
# Any LPPL version and any W3C licence variant counts.
# CLAUDE.md rule 7's data-file exception (never for code).
DATA_ONLY = ('CC-BY-4.0', 'CC-BY-SA-3.0', 'FSFAP', 'LicenseRef-hyph-bg', 'LicenseRef-hyph-sa',
             'LicenseRef-Sublime-Packages', 'LicenseRef-US-Government-Works')
DATA_ONLY_PREFIXES = ('LPPL', 'W3C')
FIRST_PARTY = 'LicenseRef-Lotus-Proprietary'

# ---------------------------------------------------------------- dependency graph

def metadata(manifest):
    r = subprocess.run(['cargo', 'metadata', '--locked', '--format-version', '1', '--manifest-path', manifest],
                       capture_output=True)
    if r.returncode:
        sys.exit('cargo metadata failed:\n' + r.stderr.decode('utf-8', 'replace'))
    return json.loads(r.stdout.decode('utf-8'))


def app_graph(meta):
    """Packages reachable through normal (non-dev, non-build) edges from the workspace members.
    Every target platform counts (macOS and Linux must build). Proc-macro crates run at build
    time, so neither they nor their dependencies are linked into the app."""
    pk = {p['id']: p for p in meta['packages']}
    nodes = {n['id']: n for n in meta['resolve']['nodes']}
    seen, stack, out = set(), list(meta['workspace_members']), []
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        if any('proc-macro' in t['kind'] for t in pk[i]['targets']):
            continue
        out.append((pk[i], set(nodes[i]['features'])))
        for d in nodes[i]['deps']:
            if any(k['kind'] is None for k in d['dep_kinds']):
                stack.append(d['pkg'])
    return sorted(out, key=lambda x: (x[0]['name'], x[0]['version']))

# ---------------------------------------------------------------- embed scan

INC = re.compile(r'\binclude(_bytes|_str|)!\s*\(\s*((?:[^()]|\([^()]*(?:\([^()]*\))*[^()]*\))*)\)')
# ponytail: unic passes its .rsv tables to another crate's macro as plain literals; generalise if others do
RSV = re.compile(r'"([^"]+\.rsv)"')
PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\s*\]')
LIT = re.compile(r'^"((?:[^"\\]|\\.)*)"\s*,?\s*$')
OUT_DIR = re.compile(r'^concat!\s*\(\s*env!\s*\(\s*"OUT_DIR"\s*\)\s*,\s*"([^"]*)"\s*,?\s*\)\s*,?\s*$')
MACRO_ARG = re.compile(r'^concat!\s*\(\s*"([^"]*)"\s*,\s*\$\w+\s*,?\s*\)$|^()\$\w+$')
DOC_PRE = re.compile(r'(doc\s*=|_doc!\s*\(|doctest!\s*\()\s*(::)?((core|std)::)?\s*$')
SKIP_DIRS = {'tests', 'benches', 'examples', 'target', '.git', 'node_modules', 'fuzz',
             'testdata', 'test-data', 'test_data'}
TESTY = re.compile(r'(^|/)(tests?|testdata|test-data|test_data|benches|examples|fixtures?)(/|$)')
BUILD_READS = re.compile(r'read_to_string|fs::read|include_str!|include_bytes!|File::open')
LIB_KINDS = {'lib', 'rlib', 'cdylib', 'staticlib', 'dylib', 'proc-macro'}


def _strip_comments(text):
    return '\n'.join('' if l.lstrip().startswith(('//', '/*', '*')) else l for l in text.split('\n'))


def scan(pkg):
    """Files that `pkg`'s library compiles into the binary as data, as paths relative to the
    crate root (generated ones as $OUT_DIR/name). Heuristic and textual: cfg(feature) gates
    are not evaluated (see `ignored` + `unless_feature` in data.json)."""
    root = os.path.dirname(pkg['manifest_path'])
    skip = {os.path.normcase(os.path.abspath(t['src_path'])) for t in pkg['targets']
            if not set(t['kind']) & LIB_KINDS}
    build_reads = False
    for t in pkg['targets']:
        if 'custom-build' in t['kind'] and os.path.exists(t['src_path']):
            with open(t['src_path'], encoding='utf-8', errors='replace') as fh:
                build_reads = bool(BUILD_READS.search(fh.read()))
    files = []
    for dp, dn, fn in os.walk(root):
        rel = os.path.relpath(dp, root).replace(os.sep, '/')
        dn[:] = sorted(d for d in dn if d not in SKIP_DIRS and not (rel == 'src' and d == 'bin'))
        for f in sorted(fn):
            p = os.path.join(dp, f)
            if (f.endswith('.rs') and f not in ('tests.rs', 'test.rs', 'build.rs')
                    and os.path.normcase(os.path.abspath(p)) not in skip):
                with open(p, 'rb') as fh:
                    b = fh.read()
                if b'include' in b or b'#[path' in b or b'.rsv"' in b:
                    files.append((dp, _strip_comments(b.decode('utf-8', 'replace'))))
    src_dir = os.path.join(root, 'src')

    def rel(path):
        return os.path.relpath(os.path.normpath(path), root).replace(os.sep, '/')

    def outside_src(path):
        p = os.path.normpath(path)  # src itself counts as inside (#[path = "."] in src/lib.rs)
        return not (p == os.path.normpath(src_dir) or p.startswith(src_dir + os.sep))

    hits, macros = set(), {}
    for dp, text in files:
        for m in INC.finditer(text):
            kind, arg = m.group(1), m.group(2).strip()
            if DOC_PRE.search(text[max(0, m.start() - 40):m.start()]):
                continue
            lm, om, mm = LIT.match(arg), OUT_DIR.match(arg), MACRO_ARG.match(arg)
            if om:
                name = om.group(1).lstrip('/\\')
                # include!() of generated Rust is only data if the build script reads inputs
                if kind or not name.endswith('.rs') or build_reads:
                    hits.add('$OUT_DIR/' + name)
            elif lm:
                tgt = os.path.join(dp, lm.group(1))
                r = rel(tgt)
                if TESTY.search(r):
                    continue
                if kind and not r.endswith('.rs'):
                    hits.add(r)
                elif not kind and (not r.endswith('.rs') or outside_src(tgt)):
                    hits.add(r)  # include!() of a data table (.rs.data, .rsv) or code kept outside src/
            elif mm:
                # include inside a crate-local macro_rules!: resolve the macro's literal call sites
                mr = list(re.finditer(r'macro_rules!\s*(\w+)', text[:m.start()]))
                if mr:
                    macros[mr[-1].group(1)] = (mm.group(1) or '', kind)
            # any other computed path is a macro parameter: it embeds the *caller's* file
        for m in RSV.finditer(text):
            hits.add(rel(os.path.join(dp, m.group(1))))
        for m in PATH_ATTR.finditer(text):
            tgt = os.path.join(dp, m.group(1))
            if outside_src(tgt) and not TESTY.search(rel(tgt)):
                hits.add(rel(tgt))
    for name, (prefix, kind) in macros.items():
        call = re.compile(r'\b' + re.escape(name) + r'!\s*\(\s*"([^"]+)"\s*\)')
        for dp, text in files:
            for m in call.finditer(text):
                if not re.search(r'macro_rules!\s*' + re.escape(name) + r'\s*\{[^}]*$', text[:m.start()][-300:]):
                    r = rel(os.path.join(dp, prefix + m.group(1)))
                    if not TESTY.search(r) and (kind or not r.endswith('.rs')):
                        hits.add(r)
    return sorted(hits)

# ---------------------------------------------------------------- manifest

def load_data():
    with open(DATA, encoding='utf-8') as fh:
        return json.load(fh)


def code_allowlist():
    with open(os.path.join(REPO, 'deny.toml'), 'rb') as fh:
        return set(tomllib.load(fh)['licenses']['allow'])


def _alts(expr):
    return [[a.strip() for a in alt.split(' AND ')] for alt in expr.split(' OR ')]


def _atom_ok(a, kind, allow):
    return a in allow or (kind == 'data' and (a in DATA_ONLY or a.startswith(DATA_ONLY_PREFIXES)))


def licence_ok(expr, kind, allow):
    return any(all(_atom_ok(a, kind, allow) for a in alt) for alt in _alts(expr))


def licence_atoms(g, allow):
    """Licences whose text the notices must carry: the first alternative the rule allows
    (we take that option), else the first alternative."""
    alts = _alts(g['licence'])
    return next((alt for alt in alts if all(_atom_ok(a, g.get('kind'), allow) for a in alt)), alts[0])


def crate_dir(meta, name, version):
    for p in meta['packages']:
        if p['name'] == name and p['version'] == version:
            return os.path.dirname(p['manifest_path'])
    return None


def registry_dir(name, version):
    """Crate source without needing it in the graph (notices must not depend on which
    Cargo.toml was scanned)."""
    home = os.environ.get('CARGO_HOME') or os.path.join(os.path.expanduser('~'), '.cargo')
    hits = sorted(glob.glob(os.path.join(home, 'registry', 'src', '*', f'{name}-{version}')))
    return hits[0] if hits else None


def local_label(g):
    loc = g.get('local')
    if not loc:
        return ''
    return f' (patched copy in {loc})' if loc.startswith('patches/') else ' (this repository)'


def source_dir(g):
    if g.get('local'):
        return os.path.join(REPO, g['local'])
    return registry_dir(g['crate'], g['version'])


def expand(g):
    d = source_dir(g)
    out = []
    for pat in g['files']:
        if pat.startswith('$OUT_DIR/'):
            out.append((pat, [pat]))
            continue
        m = sorted(os.path.relpath(p, d).replace(os.sep, '/')
                   for p in glob.glob(os.path.join(d, pat), recursive=True) if os.path.isfile(p))
        out.append((pat, m))
    return out

# ---------------------------------------------------------------- attribution extractors

def _cbor(b, o=0):
    ib = b[o]; mt, ai = ib >> 5, ib & 31; o += 1
    if ai < 24: n = ai
    elif ai == 31: n = None
    else:
        k = 1 << (ai - 24); n = int.from_bytes(b[o:o + k], 'big'); o += k
    if mt in (0, 1): return (n if mt == 0 else -1 - n), o
    if mt in (2, 3):
        v = b[o:o + n]; o += n
        return (v.decode('utf-8', 'replace') if mt == 3 else v), o
    if mt in (4, 5):
        out = [] if mt == 4 else {}
        i = 0
        while (n is None and b[o] != 0xff) or (n is not None and i < n):
            if mt == 4:
                v, o = _cbor(b, o); out.append(v)
            else:
                k, o = _cbor(b, o); v, o = _cbor(b, o); out[k] = v
            i += 1
        if n is None: o += 1
        return out, o
    if mt == 6: return _cbor(b, o)
    return {20: False, 21: True, 22: None}.get(ai, n), o


def _val(x, *keys):
    for k in keys:
        if isinstance(x, dict) and k in x:
            x = x[k]
    return x if isinstance(x, str) else ''


def csl_attribution(path):
    d, _ = _cbor(open(path, 'rb').read())
    info = d.get('info', {}) if isinstance(d, dict) else {}
    names = lambda k: '; '.join(p.get('name', '') for p in info.get(k, []) if isinstance(p, dict))
    rights = info.get('rights', {})
    lic = rights.get('@license', '') if isinstance(rights, dict) else ''
    parts = []
    if d.get('@xml:lang'):
        parts.append('CSL locale ' + d['@xml:lang'])
    else:
        parts.append('"%s"' % _val(info.get('title', {}), '$value'))
        if info.get('id'):
            parts.append(info['id'])
    for k, lbl in (('author', 'authors'), ('contributor', 'contributors'), ('translator', 'translators')):
        if names(k):
            parts.append(f'{lbl}: {names(k)}')
    parts.append('licence: ' + (lic or 'none stated in file'))
    return ', '.join(parts), lic


def two_face_acknowledgements(path):
    d = zlib.decompress(open(path, 'rb').read()); o = 0
    types = ['Sublime', 'MIT', 'BSD-2-Clause', 'BSD-2-Clause (FreeBSD)', 'Unlicense', 'BSD-3-Clause',
             'Apache-2.0', 'WTFPL']

    def u(n):
        nonlocal o; v = int.from_bytes(d[o:o + n], 'little'); o += n; return v

    def s():
        n = u(8); nonlocal o; v = d[o:o + n].decode('utf-8'); o += n; return v
    out = []
    for kind in ('syntax', 'theme'):
        for _ in range(u(8)):
            ty = types[u(4)]; text = s(); rel = s()
            out.append((kind, rel, ty, text))
    return out

# ---------------------------------------------------------------- generate

def wrap(text, indent='  '):
    return '\n'.join((indent + l).rstrip() for l in text.strip('\n').split('\n'))


def fill(text, indent='  '):
    """Wrap each paragraph line to 78 columns; blank lines kept."""
    return '\n'.join(textwrap.fill(l, 78, initial_indent=indent, subsequent_indent=indent) if l.strip() else ''
                     for l in text.strip('\n').split('\n'))


def licence_text(data, lid):
    spec = data['licences'].get(lid)
    if spec is None:
        return None
    parts = []
    if 'url' in spec:
        parts.append('Canonical text: ' + spec['url'])
    if 'note' in spec:
        parts.append(spec['note'])
    if 'crate' in spec:
        if 'local' in spec:
            d = os.path.join(REPO, spec['local'])
        elif 'version' in spec:
            d = registry_dir(spec['crate'], spec['version'])
        else:
            d = os.path.join(REPO, spec['crate'])
        with open(os.path.join(d, spec['path']), encoding='utf-8', errors='replace') as fh:
            t = fh.read().replace('\r\n', '\n')
        if 'from' in spec:
            t = t[t.index(spec['from']):]
        if 'to' in spec:
            t = t[:t.index(spec['to'])]
        parts.append(f"Text as shipped in {spec['crate']} {spec.get('version', '')} {spec['path']}:".replace('  ', ' ')
                     + '\n\n' + t.strip('\n'))
    return '\n\n'.join(parts)


def generate(data):
    L = ['Third-party notices for data files compiled into the Lotus application binary',
         '(fonts, colour profiles, hyphenation patterns, citation styles, syntax definitions and',
         'similar). Rust crates used as code are covered by their own licences via cargo-deny.',
         'Generated by tools/notices/notices.py from tools/notices/data.json; do not edit by hand.', '']
    used, allow = set(), code_allowlist()
    for g in data['groups']:
        used.update(licence_atoms(g, allow))
        L += ['=' * 78, f"[{g['id']}] {g['title']}", '=' * 78,
              f"Crate:   {g['crate']} {g['version']}" + local_label(g),
              f"Licence: {g['licence']}", 'What:', fill(g['what']), 'Files:']
        for pat, files in expand(g):
            L += ['  ' + f for f in files] or ['  ' + pat]
        L += ['Attribution:', fill(g['attribution'])]
        if g.get('attribution_file'):
            with open(os.path.join(source_dir(g), g['attribution_file']), encoding='utf-8') as fh:
                L += [f"  Licence notice as shipped ({g['attribution_file']}):", '',
                      fill(fh.read().replace('\r\n', '\n'), '    ')]
        if g.get('verified') is not True:
            L += ['Verification:', fill(str(g.get('verified')))]
        ex = g.get('extract')
        if ex == 'csl':
            L.append('Per-file attribution (from the metadata embedded in each file):')
            for _, files in expand(g):
                for f in files:
                    L.append('  ' + os.path.basename(f) + ': ' + csl_attribution(os.path.join(source_dir(g), f))[0])
        elif ex == 'two-face':
            L.append('Upstream licence notices for the embedded syntax and theme definitions (from')
            L.append('generated/acknowledgements_full.bin, which is itself embedded):')
            for kind, rel, ty, text in two_face_acknowledgements(os.path.join(source_dir(g), g['extract_path'])):
                L += ['', f'  --- {kind}: {rel} ({ty})', wrap(text, '    ')]
        L.append('')
    L += ['=' * 78, 'Licence texts', '=' * 78, '']
    for lid in sorted(used):
        t = licence_text(data, lid)
        L += ['-' * 78, lid, '-' * 78, t if t is not None else '(no text recorded)', '']
    return '\n'.join(L).rstrip('\n') + '\n'

# ---------------------------------------------------------------- check

def unexplained(g, pat):
    """Why an `ignored` entry is not acceptable (empty if it is). An ignore must be either
    behind a feature (`unless_feature`, checked to be off below) or test-only: `test_only`
    names the source file that includes it, and the include must come after a #[cfg(test)]
    or #[test] in that file."""
    if g.get('unless_feature'):
        return []
    t = g.get('test_only')
    if not t:
        return ['unexplained: give unless_feature or test_only (or list it under first_party)']
    d = registry_dir(g['crate'], g['version'])
    if not d:
        return []  # source not downloaded here; the version check reports a mismatch
    path = os.path.join(d, t)
    if not os.path.exists(path):
        return [f'test_only file {t} does not exist']
    text = open(path, encoding='utf-8', errors='replace').read()
    stem = pat.split('*')[0].lstrip('./')
    for m in re.finditer(r'include(_bytes|_str|)!', text):
        line = text[m.start():text.find('\n', m.start())]
        if stem in line or stem in text[m.start():m.start() + 200]:
            before = text[:m.start()]
            if '#[cfg(test)]' in before or '#[test]' in before:
                return []
            return [f'{t} includes it outside #[cfg(test)] / #[test]']
    return [f'no include of {stem} found in {t}']


def check(data, meta, strict):
    errors, warnings = [], []
    allow = code_allowlist()
    graph = app_graph(meta)
    versions = {}
    for p in meta['packages']:
        versions.setdefault(p['name'], set()).add(p['version'])
    features = {(p['name'], p['version']): f for p, f in graph}

    ids = [g['id'] for g in data['groups']]
    errors += [f'duplicate group id {i}' for i in sorted({i for i in ids if ids.count(i) > 1})]
    for g in data['groups'] + data['ignored'] + data['first_party']:
        tag = g.get('id') or f"{'ignored' if g in data['ignored'] else 'first-party'} {g['crate']}"
        if g['version'] not in versions.get(g['crate'], ()):
            errors.append(f"{tag}: manifest says {g['crate']} {g['version']}, Cargo.lock has "
                          f"{', '.join(sorted(versions.get(g['crate'], []))) or 'no such crate'}")
    for g in data['groups']:
        if not licence_ok(g['licence'], g.get('kind'), allow):
            msg = f"{g['id']}: licence '{g['licence']}' ({g.get('kind')}) is outside CLAUDE.md rule 7"
            if g.get('needs_owner_decision') and not strict:
                warnings.append(msg + ' -- flagged for the owner: ' + g['needs_owner_decision'])
            else:
                errors.append(msg)
        for a in licence_atoms(g, allow):
            if a not in data['licences']:
                errors.append(f"{g['id']}: no licence text recorded for '{a}' in data.json 'licences'")
        if registry_dir(g['crate'], g['version']) or g.get('local'):
            for pat, files in expand(g):
                if not files:
                    errors.append(f"{g['id']}: glob '{pat}' matches nothing in {g['crate']} {g['version']}")
        if g.get('extract') == 'csl':
            for _, files in expand(g):
                for f in files:
                    lic = csl_attribution(os.path.join(source_dir(g), f))[1]
                    if lic and 'by-sa/3.0' not in lic:
                        errors.append(f"{g['id']}: {f} states licence {lic}")

    for g in data['first_party']:
        if g.get('licence') != FIRST_PARTY:
            errors.append(f"first-party {g['crate']}: licence must be {FIRST_PARTY}")
        d = crate_dir(meta, g['crate'], g['version'])
        for pat in g['files']:
            if d and not glob.glob(os.path.join(d, pat)):
                errors.append(f"first-party {g['crate']}: '{pat}' matches nothing")
    for g in data['ignored']:
        errors += [f"ignored {g['crate']} {pat}: {e}" for pat in g['files'] for e in unexplained(g, pat)]

    def matches(entries, p, f):
        return [g for g in entries if g['crate'] == p['name'] and g['version'] == p['version']
                and any(fnmatch.fnmatchcase(f, pat) for pat in g['files'])]

    used_ignores = set()
    for p, feats in graph:
        for f in scan(p):
            if matches(data['groups'] + data['first_party'], p, f):
                continue
            ign = matches(data['ignored'], p, f)
            used_ignores.update(id(g) for g in ign)
            live = [g for g in ign if g.get('unless_feature') in feats]
            if ign and not live:
                continue
            why = (f" (ignored only while feature '{live[0]['unless_feature']}' is off, and it is on)" if live else '')
            errors.append(f"{p['name']} {p['version']} embeds {f}, which no data.json entry covers{why}")

    # An ignore that no longer matches anything the scan finds is stale: it could silently
    # cover a new file later.
    for g in data['ignored']:
        if id(g) not in used_ignores and g['crate'] in {p['name'] for p, _ in graph}:
            errors.append(f"ignored {g['crate']} {g['files']}: matches nothing the scan finds; remove it")

    # Data excluded by decision must not come back (a hypher update without the patch, or a
    # feature switched on).
    for fb in data.get('forbidden', []):
        for p, feats in graph:
            if p['name'] != fb['crate']:
                continue
            d = os.path.dirname(p['manifest_path'])
            for f in fb['files']:
                if os.path.exists(os.path.join(d, f)) or f in scan(p):
                    errors.append(f"{p['name']} {p['version']} contains forbidden {f}: {fb['reason']}")
            for feat in fb['features']:
                if feat in feats:
                    errors.append(f"{p['name']} {p['version']} has forbidden feature '{feat}' on: {fb['reason']}")

    want = generate(data)
    have = open(OUT, encoding='utf-8').read() if os.path.exists(OUT) else ''
    if have.replace('\r\n', '\n') != want:
        errors.append('THIRD_PARTY_NOTICES.txt is stale: run  py tools/notices/notices.py generate')
    for w in warnings:
        print('WARNING:', w)
    for e in errors:
        print('ERROR:', e)
    print(f'notices check: {len(graph)} crates scanned, {len(errors)} error(s), {len(warnings)} flagged')
    return 1 if errors else 0


def main(argv):
    if not argv or argv[0] not in ('generate', 'check', 'scan'):
        sys.exit(__doc__)
    manifest = os.path.join(REPO, 'Cargo.toml')
    if '--manifest-path' in argv:
        manifest = argv[argv.index('--manifest-path') + 1]
    data = load_data()
    if argv[0] == 'generate':
        with open(OUT, 'w', encoding='utf-8', newline='\n') as fh:
            fh.write(generate(data))
        print('wrote', OUT)
        return 0
    meta = metadata(manifest)
    if argv[0] == 'scan':
        for p, feats in app_graph(meta):
            for f in scan(p):
                print(p['name'], p['version'], f)
        return 0
    return check(data, meta, '--strict' in argv)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
