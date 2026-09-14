#!/usr/bin/env python3
"""Offline main-shell journey using the existing supervised fixture; no live accounts."""
import argparse
import importlib.util
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('workspace_journey_base', ROOT / 'helm/tests/ux272_journeys.py')
u = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = u
spec.loader.exec_module(u)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    fixture = u.Fixture(args.bin_dir.resolve())
    journey = u.Journey(fixture, args.bin_dir.resolve(), output)
    try:
        fixture.start()
        fixture.seed_account()
        session = fixture.session()
        ui = journey.connect()
        for width, height in [(120, 40), (40, 18)]:
            ui.resize(width, height)
            u.settle(ui)
            text = ui.screen.text()
            assert 'Helm ▾' in text and '+ New' in text and 'Find' in text, text
            assert all(old not in text for old in ['F1 Help', 'F2 Voyages', 'F3 Console', 'F8 Explore', 'F9 Actions']), text
            u.paste(ui, 'retained main-shell draft')
            u.settle(ui)
            ui.send(b'\x10')  # Ctrl+P opens named menu, not model input
            ui.until(lambda s: 'Connections' in s.text(), 'named Helm menu')
            (output / f'menu-{width}.txt').write_text(ui.screen.text())
            u.paste(ui, 'must not enter composer')
            ui.send(b'\x1b')
            u.settle(ui)
            assert 'retained main-shell' in ui.screen.text(), ui.screen.text()
            assert 'must not enter composer' not in ui.screen.text()
            assert not fixture.snapshot(session)['messages']
            ui.send(b'\x10\x1b[B\r')  # Menu -> Find -> named conversation picker
            ui.until(lambda s: 'Find voyages' in s.text(), 'named conversation picker')
            ui.send(b'\x1b')
            u.settle(ui)
            # Preferences is one options surface, not four permanent composer fields.
            ui.send(b'\x10' + b'\x1b[B' * 3 + b'\r')
            ui.until(lambda screen: 'Model options' in screen.text(), 'combined model/account options')
            assert all(label in ui.screen.text() for label in ['Account:', 'Thinking:', 'Service:'])
            (output / f'preferences-{width}.txt').write_text(ui.screen.text())
            u.paste(ui, 'must not become settings')
            ui.send(b'\x1b'); u.settle(ui)
            assert 'retained main-shell' in ui.screen.text()
            ui.send(b'\x10' + b'\x1b[B' * 2 + b'\r')
            ui.until(lambda screen: 'Copy last answer' in screen.text(), 'conversation work menu')
            assert 'Delegated work' in ui.screen.text()
            (output / f'work-{width}.txt').write_text(ui.screen.text())
            ui.send(b'\x1b'); u.settle(ui)
            assert not fixture.snapshot(session)['messages']
            ui.send(b'\x01\x18')  # clear only the synthetic draft
            u.settle(ui)
            (output / f'main-{width}.txt').write_text(ui.screen.text())
        ui.resize(40, 18)
        fixture.command(session, fixture.submit(session, 'UXQUESTION'))
        ui.until(lambda screen: 'Your answer needed' in screen.text(), 'narrow request review')
        u.settle(ui)
        (output / 'question-40.txt').write_text(ui.screen.text())
        assert 'Synthetic choice' in ui.screen.text(), ui.screen.text()
        ui.send(b'\x1b[B\r')
        journey.finish(session)
        ui.until(lambda screen: 'Model:' in screen.text(), 'composer restored after answer')
        journey.note('At 40x18 the request uses the available width, shows question and choices, accepts a scoped answer and restores composition.')
        journey.note('Named main controls and Ctrl+P menu at 120x40 and 40x18; menu paste cannot submit/edit draft; Find opens existing identity picker; combined model/account options and conversation work menu preserve draft; no message dispatch.')
        (output / 'result.json').write_text(json.dumps({'status':'passed','observations':journey.observations,'provider':'synthetic loopback only'}, indent=2))
    finally:
        errors = journey.close()
        if errors:
            raise RuntimeError(errors)


if __name__ == '__main__':
    main()
