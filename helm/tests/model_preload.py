#!/usr/bin/env python3
"""Offline model metadata is fetched before click; warm opens need no HTTP wait."""
import argparse
import importlib.util
import json
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('preload_fixture', ROOT / 'helm/tests/ux272_journeys.py')
u = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = u
spec.loader.exec_module(u)


class Models(u.Provider):
    def do_GET(self):
        self.server.catalog_requests += 1
        self.server.catalog_paths.append(self.path)
        # Make a synchronous-on-click fetch visibly too slow for the assertion.
        time.sleep(1)
        data = json.dumps({'data': [{'id': 'fixture-model'}, {'id': 'preloaded-model'}]}).encode()
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            self.server.catalog_completed += 1
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    bins, out = args.bin_dir.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    u.Provider = Models
    f = u.Fixture(bins)
    f.provider.catalog_requests = 0
    f.provider.catalog_completed = 0
    f.provider.catalog_paths = []
    j = u.Journey(f, bins, out)
    try:
        f.start()
        f.seed_account()
        sid = f.session()
        startup_requests = f.provider.catalog_requests
        startup_completed = f.provider.catalog_completed
        ui = j.connect()
        before = f.snapshot(sid)
        deadline = time.monotonic() + 10
        while f.provider.catalog_completed == startup_completed and time.monotonic() < deadline:
            ui.pump()
            time.sleep(.02)
        assert f.provider.catalog_completed > startup_completed, 'no metadata preload before opening chooser'
        u.settle(ui)
        u.paste(ui, 'preserve preload draft')
        u.settle(ui)
        warm_requests = f.provider.catalog_requests
        times = []
        for _ in range(2):
            rows = ui.screen.text().splitlines()
            row = next(i for i, text in enumerate(rows) if 'Model:' in text)
            col = rows[row].index('Model:')
            started = time.monotonic()
            ui.send(f'\x1b[<0;{col+1};{row+1}M\x1b[<0;{col+1};{row+1}m'.encode())
            ui.until(lambda s: 'Choose a model' in s.text() and 'preloaded-model' in s.text(),
                     'cached model immediately visible', timeout=.75)
            times.append(time.monotonic() - started)
            text = ui.screen.text()
            assert 'loading…' not in text
            assert '[Refresh]' in text and '[Retry]' not in text
            assert 'Applies to your next message' in text
            assert 'provider-authoritative' not in text and 'entitlements' not in text
            assert 'This computer ·' not in text and 'Next message only' not in text
            (out / f'warm-{len(times)}.txt').write_text(ui.screen.text())
            ui.send(b'\x1b')
            u.settle(ui)
            assert 'preserve preload draft' in ui.screen.text()
        assert f.provider.catalog_requests == warm_requests, f'warm open unexpectedly refetched models: {f.provider.catalog_requests}'
        assert f.snapshot(sid)['revision'] == before['revision']
        assert not f.snapshot(sid)['messages']
        assert not f.provider.bodies
        (out / 'result.json').write_text(json.dumps({
            'status': 'passed', 'warm_open_seconds': times,
            'catalogue_http_requests_before_click': warm_requests,
            'catalogue_http_requests_after_click': f.provider.catalog_requests - warm_requests,
            'catalogue_paths': f.provider.catalog_paths,
            'checks': ['metadata fetched before click', 'first and repeated warm opens under 750ms',
                       'no loading placeholder on warm open', 'no repeated HTTP',
                       'draft and revision retained', 'no inference']}, indent=2))
    finally:
        errors = j.close()
        if errors:
            raise RuntimeError(errors)


if __name__ == '__main__':
    main()
