import { promises as fs } from 'fs';
import path from 'path';

async function deepMerge(target: Record<string, any>, source: Record<string, any>): Promise<Record<string, any>> {
  for (const key of Object.keys(source)) {
    const value = source[key];
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      target[key] = await deepMerge(target[key] || {}, value);
    } else {
      target[key] = value;
    }
  }
  return target;
}

async function buildFor(browser: string) {
  const basePath = path.join('extension', 'src', 'manifest.base.json');
  const browserPath = path.join('extension', 'src', `manifest.${browser}.json`);
  const distDir = path.join('extension', `dist-${browser}`);
  const base = JSON.parse(await fs.readFile(basePath, 'utf8'));
  const overrides = JSON.parse(await fs.readFile(browserPath, 'utf8'));
  const merged = await deepMerge(base, overrides);
  await fs.rm(distDir, { recursive: true, force: true });
  await fs.mkdir(distDir, { recursive: true });
  await fs.writeFile(path.join(distDir, 'manifest.json'), JSON.stringify(merged, null, 2));
  await copyBuiltFiles(browser);
}

async function main() {
  await Promise.all(['chrome', 'firefox'].map(buildFor));
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});

async function copyBuiltFiles(browser: string) {
  const distDir = path.join('extension', `dist-${browser}`);
  const sourceDir = path.join('extension', 'dist');
  await copyDir(sourceDir, distDir);
}

async function copyDir(sourceDir: string, targetDir: string) {
  const entries = await fs.readdir(sourceDir, { withFileTypes: true });
  await fs.mkdir(targetDir, { recursive: true });

  for (const entry of entries) {
    const sourcePath = path.join(sourceDir, entry.name);
    const targetPath = path.join(targetDir, entry.name);

    if (entry.isDirectory()) {
      await copyDir(sourcePath, targetPath);
      continue;
    }

    if (entry.isFile()) {
      await fs.copyFile(sourcePath, targetPath);
    }
  }
}
