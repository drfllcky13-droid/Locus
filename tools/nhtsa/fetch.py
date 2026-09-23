"""Fetch NHTSA full-width frontal rigid-barrier crash tests for the CRASH3 stiffness table.

    py tools/nhtsa/fetch.py --cache DIR [--out crates/locus-analysis/data/nhtsa-frontal-barrier.csv]

Source: the NHTSA Vehicle Crash Test Database API (https://nrd.api.nhtsa.dot.gov/, public
domain, a work of the US Government). Every test is listed (by test-number range); those with the configuration
"VEHICLE INTO BARRIER", an impact angle of 0°, no offset, and a rigid flat barrier are kept,
and each vehicle's test mass, speed, width and crush profile (C1–C6) are written, one row per
test vehicle. Raw responses are cached in DIR (one JSON file per request), so a run resumes.
Nothing is derived here: the Campbell coefficients are computed by locus-analysis from this
file (crates/locus-analysis/src/stiffness.rs; docs/methods/crash-stiffness.md). Stdlib only.
"""
import argparse, concurrent.futures, csv, json, os, sys, time, urllib.request

BASE = 'https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-database-test-results'
UA = {'User-Agent': 'Locus crash-stiffness table builder (forensic reconstruction; stdlib urllib)'}
COLUMNS = ['test_no', 'test_date', 'test_type', 'test_reference', 'vehicle_no', 'make', 'model',
           'model_year', 'body_type', 'mass_kg', 'speed_kmh', 'vehicle_width_mm',
           'damage_width_mm', 'pdof_deg', 'c1_mm', 'c2_mm', 'c3_mm', 'c4_mm', 'c5_mm', 'c6_mm',
           'vdi', 'barrier']


def get(cache, name, url):
    path = os.path.join(cache, name + '.json')
    if os.path.exists(path):
        with open(path, encoding='utf-8') as f:
            return json.load(f)
    for attempt in range(5):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120) as r:
                data = json.loads(r.read().decode('utf-8'))
            break
        except Exception as e:  # network hiccups: back off and retry
            if attempt == 4:
                raise RuntimeError(f'{url}: {e}')
            time.sleep(2 ** attempt)
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(data, f)
    return data


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cache', required=True)
    ap.add_argument('--out', default='crates/locus-analysis/data/nhtsa-frontal-barrier.csv')
    a = ap.parse_args()
    os.makedirs(a.cache, exist_ok=True)
    # The API's paging repeats and skips rows, so tests are listed by test-number range
    # (500 at a time), each range in one page, until five empty ranges in a row.
    tests, lo, empty = {}, 1, 0
    while empty < 5:
        hi = lo + 499
        d = get(a.cache, f'range-{lo}', f'{BASE}/by-search?count=1000&pageNumber=0&testNoFrom={lo}&testNoTo={hi}&orderBy=testNo&sortBy=ASC')
        rows = d.get('results') or []
        if len(rows) >= 1000:
            raise RuntimeError(f'range {lo}-{hi} is full; use smaller ranges')
        for r in rows:
            tests[r['testNo']] = r
        empty = 0 if rows else empty + 1
        print(f'tests {lo}-{hi}: {len(rows)} ({len(tests)} so far)', file=sys.stderr)
        lo = hi + 1
    tests = [tests[k] for k in sorted(tests)]
    frontal = [t for t in tests if t.get('testConfiguration') == 'VEHICLE INTO BARRIER'
               and str(t.get('impactAngle')) == '0' and not t.get('offsetDistance')]
    print(f'{len(frontal)} full-width frontal barrier tests of {len(tests)}', file=sys.stderr)

    def one(t):
        n = t['testNo']
        barrier = (get(a.cache, f'barrier-{n}', f'{BASE}/get-barrier-info/{n}').get('results') or [{}])[0]
        if barrier.get('rigidOrDeformableBarrier') != 'RIGID' or barrier.get('barrierShape') != 'FLAT BARRIER':
            return []
        detail = (get(a.cache, f'test-{n}', f'{BASE}/get-test-detail/{n}').get('results') or [{}])[0]
        vehicles = get(a.cache, f'vehicles-{n}', f'{BASE}/get-vehicle-info/{n}').get('results') or []
        out = []
        for v in vehicles:
            vn = v['vehicleNo']
            r = (get(a.cache, f'vehicle-{n}-{vn}', f'{BASE}/get-vehicle-detail-info/{vn}/{n}').get('results') or [{}])[0]
            out.append({
                'test_no': n, 'test_date': (detail.get('testDate') or '')[:10],
                'test_type': t.get('testType'), 'test_reference': t.get('testReferenceNo'),
                'vehicle_no': vn, 'make': r.get('vehicleMake'), 'model': r.get('vehicleModel'),
                'model_year': r.get('modelYear'), 'body_type': r.get('bodyType'),
                'mass_kg': r.get('vehicleTestWeight'), 'speed_kmh': r.get('vehicleSpeed') or t.get('closingSpeed'),
                'vehicle_width_mm': r.get('vehicleWidth'), 'damage_width_mm': r.get('totalLengthofIndentation'),
                'pdof_deg': r.get('principalDirectionofForce'),
                **{f'c{k}_mm': r.get(f'damageProfileDistances{w}') for k, w in
                   zip(range(1, 7), ['One', 'Two', 'Three', 'Four', 'Five', 'Six'])},
                'vdi': (r.get('vehicleDamageIndex') or '').strip(), 'barrier': 'rigid flat',
            })
        return out

    rows = []
    with concurrent.futures.ThreadPoolExecutor(4) as ex:
        for k, got in enumerate(ex.map(one, frontal)):
            rows += got
            if k % 100 == 0:
                print(f'{k} of {len(frontal)} tests, {len(rows)} vehicles', file=sys.stderr)
    rows.sort(key=lambda r: (r['test_no'], r['vehicle_no']))
    with open(a.out, 'w', encoding='utf-8', newline='') as f:
        f.write('# NHTSA Vehicle Crash Test Database (https://nrd.api.nhtsa.dot.gov/), a public-domain\n')
        f.write('# work of the US Government. Full-width frontal rigid flat-barrier tests; one row per\n')
        f.write(f'# test vehicle, as published (mm, kg, km/h). Fetched {time.strftime("%Y-%m-%d")} by\n')
        f.write('# tools/nhtsa/fetch.py. Coefficients are derived by locus-analysis (stiffness.rs).\n')
        w = csv.DictWriter(f, COLUMNS)
        w.writeheader()
        w.writerows(rows)
    print(f'wrote {len(rows)} rows to {a.out}', file=sys.stderr)


if __name__ == '__main__':
    main()
