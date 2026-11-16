import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import process from 'node:process';

const rustPath = 'crates/mine-utils-core/src/error_code.rs';
const generatedPath = 'src/lib/generated/wire.ts';
const rust = readFileSync(rustPath, 'utf8');
const generated = readFileSync(generatedPath, 'utf8');

function run(args, expectedStatus = 0) {
  const result = spawnSync('pnpm', args, { encoding: 'utf8' });
  const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
  process.stdout.write(output);
  assert.equal(result.error, undefined);
  if (expectedStatus === 0) assert.equal(result.status, 0, args.join(' '));
  else {
    process.stdout.write(`Mutation tsc exit ${result.status}; first error: ${output.split('\n').find((line) => line.includes('error TS')) ?? 'none'}\n`);
    assert.ok(result.status !== null && result.status !== 0, args.join(' '));
  }
  return output;
}

run(['wire:check']);
run(['typecheck']);
for (const [variant, code, expectedError] of [
  ['NoActiveGameDir', 'no_active_game_dir', /ipc\.mock\.ts.*error TS2345.*no_active_game_dir/],
  ['DownloadOffline', 'download_offline', /useModrinth\.ts.*error TS2367.*download_offline/],
  ['GameRunning', 'game_running', /error TS2367.*game_running/],
  ['StaleStack', 'stale_stack', /error TS2367.*stale_stack/],
  ['WorldMismatch', 'world_mismatch', /error TS2367.*world_mismatch/],
]) {
  try {
    const renamed = rust.replace(`${variant} => "${code}"`, `${variant} => "${code}_renamed"`);
    assert.notEqual(renamed, rust);
    writeFileSync(rustPath, renamed);
    run(['wire:generate']);
    const failure = run(['typecheck'], 1);
    assert.match(failure, expectedError);
    process.stdout.write(`PASS: renaming ${code} rejects its unchanged consumer\n`);
  } finally {
    writeFileSync(rustPath, rust);
    writeFileSync(generatedPath, generated);
  }
}
try {
  writeFileSync(generatedPath, `${generated}\n// Deliberately stale acceptance probe.\n`);
  assert.match(run(['wire:check'], 1), /wire\.ts is stale/);
  process.stdout.write('PASS: the freshness check rejects a stale generated file\n');
} finally {
  writeFileSync(generatedPath, generated);
}
run(['wire:check']);
run(['typecheck']);
