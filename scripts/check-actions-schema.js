const { readFileSync } = require('fs');
const { createHash } = require('crypto');
const { join } = require('path');

const repoRoot = join(__dirname, '..');
const schemaPath = join(repoRoot, 'schema', 'actions-v1.json');
const typesPath = join(repoRoot, 'extension', 'src', 'types', 'actions.ts');

const schemaContent = readFileSync(schemaPath, 'utf8');
const schemaHash = createHash('sha256').update(schemaContent).digest('hex');
const schema = JSON.parse(schemaContent);
const schemaVersion = schema.schema_version ?? 'unknown';

const typesContent = readFileSync(typesPath, 'utf8');
const hashLine = typesContent.split('\n').find(line => line.startsWith('// schema-sha256:'));
const versionLine = typesContent.split('\n').find(line => line.startsWith('// schema-version:'));

if (!hashLine || !versionLine) {
  console.error('actions.ts missing schema metadata header. Re-run generate-types.');
  process.exit(1);
}

const recordedHash = hashLine.replace('// schema-sha256:', '').trim();
const recordedVersion = versionLine.replace('// schema-version:', '').trim();

if (recordedHash !== schemaHash) {
  console.error(`actions.ts schema hash mismatch.\n  expected: ${schemaHash}\n  found:    ${recordedHash}`);
  process.exit(1);
}

if (recordedVersion !== schemaVersion) {
  console.error(`actions.ts schema version mismatch.\n  expected: ${schemaVersion}\n  found:    ${recordedVersion}`);
  process.exit(1);
}

console.log('actions.ts matches schema/actions-v1.json');
