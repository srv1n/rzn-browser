import { promises as fs } from 'fs';
import crypto from 'node:crypto';
import path from 'path';

type JsonRecord = Record<string, any>;

export const CHROMIUM_EXTENSION_TARGETS = ['chrome', 'edge', 'chromium'] as const;
export const EXTENSION_BUILD_TARGETS = ['chrome', 'firefox', 'edge', 'chromium'] as const;
export const RZN_NATIVE_HOST_NAME = 'com.rzn.browser.broker';
export const RZN_DEV_EXTENSION_ID = 'bogjdnehdficgkhklinmnbgiiofbamji';
export const RZN_DEV_EXTENSION_ORIGIN = `chrome-extension://${RZN_DEV_EXTENSION_ID}/`;

export type ExtensionBuildTarget = (typeof EXTENSION_BUILD_TARGETS)[number];

type BuildOptions = {
  rootDir?: string;
  distSourceDir?: string;
  distRootDir?: string;
  layout?: 'legacy' | 'nested';
  copyFiles?: boolean;
  buildSignature?: string;
};

export function deepMerge(target: JsonRecord, source: JsonRecord): JsonRecord {
  for (const key of Object.keys(source)) {
    const value = source[key];
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      target[key] = deepMerge(target[key] || {}, value);
    } else {
      target[key] = value;
    }
  }
  return target;
}

export function buildManifest(base: JsonRecord, overrides: JsonRecord): JsonRecord {
  return deepMerge(structuredClone(base), structuredClone(overrides));
}

export function buildMetadataForTarget(
  browser: ExtensionBuildTarget,
  buildSignatureValue = buildSignature()
): JsonRecord {
  return {
    schema_version: 1,
    extension_target: browser,
    browser_family:
      (CHROMIUM_EXTENSION_TARGETS as readonly string[]).includes(browser) ? 'chromium' : browser,
    extension_id: RZN_DEV_EXTENSION_ID,
    extension_origin: RZN_DEV_EXTENSION_ORIGIN,
    native_host_name: RZN_NATIVE_HOST_NAME,
    build_signature: buildSignatureValue,
  };
}

export function buildSignature(): string {
  const value = process.env.RZN_BUILD_SIGNATURE?.trim();
  return value && value.length > 0 ? value : 'dev-unknown';
}

export function chromeExtensionIdFromManifestKey(key: string): string {
  const der = Buffer.from(key, 'base64');
  const digest = crypto.createHash('sha256').update(der).digest();
  return [...digest.subarray(0, 16)]
    .map((byte) => String.fromCharCode(97 + (byte >> 4)) + String.fromCharCode(97 + (byte & 0x0f)))
    .join('');
}

export async function buildManifestForTarget(
  browser: ExtensionBuildTarget,
  rootDir = process.cwd()
): Promise<JsonRecord> {
  const basePath = path.join(rootDir, 'extension', 'src', 'manifest.base.json');
  const browserPath = path.join(rootDir, 'extension', 'src', `manifest.${browser}.json`);
  const base = JSON.parse(await fs.readFile(basePath, 'utf8'));
  const overrides = JSON.parse(await fs.readFile(browserPath, 'utf8'));
  return buildManifest(base, overrides);
}

export async function buildFor(browser: ExtensionBuildTarget, options: BuildOptions = {}) {
  const rootDir = options.rootDir ?? process.cwd();
  const distRootDir = options.distRootDir ?? path.join(rootDir, 'extension');
  const distDir =
    options.layout === 'nested'
      ? path.join(distRootDir, browser)
      : path.join(distRootDir, `dist-${browser}`);
  const merged = await buildManifestForTarget(browser, rootDir);
  await fs.rm(distDir, { recursive: true, force: true });
  await fs.mkdir(distDir, { recursive: true });
  await fs.writeFile(path.join(distDir, 'manifest.json'), JSON.stringify(merged, null, 2));
  await fs.writeFile(
    path.join(distDir, 'rzn-build.json'),
    JSON.stringify(buildMetadataForTarget(browser, options.buildSignature), null, 2)
  );
  if (options.copyFiles !== false) {
    await copyBuiltFiles(browser, options);
  }
}

async function main() {
  const args = parseCliArgs(process.argv.slice(2));
  await Promise.all(args.targets.map((target) => buildFor(target, args.options)));
}

export async function copyBuiltFiles(browser: ExtensionBuildTarget, options: BuildOptions = {}) {
  const rootDir = options.rootDir ?? process.cwd();
  const distRootDir = options.distRootDir ?? path.join(rootDir, 'extension');
  const distDir =
    options.layout === 'nested'
      ? path.join(distRootDir, browser)
      : path.join(distRootDir, `dist-${browser}`);
  const sourceDir = options.distSourceDir ?? path.join(rootDir, 'extension', 'dist');
  await copyDir(sourceDir, distDir);
}

export async function copyDir(sourceDir: string, targetDir: string) {
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

function parseCliArgs(argv: string[]): { targets: ExtensionBuildTarget[]; options: BuildOptions } {
  let target: ExtensionBuildTarget | 'all' = 'all';
  const options: BuildOptions = {};

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = () => {
      const value = argv[index + 1];
      if (!value) {
        throw new Error(`Missing value for ${arg}`);
      }
      index += 1;
      return value;
    };

    if (arg === '--target') {
      target = parseTarget(next());
    } else if (arg === '--source-dir') {
      options.distSourceDir = path.resolve(next());
    } else if (arg === '--dist-root') {
      options.distRootDir = path.resolve(next());
    } else if (arg === '--layout') {
      const layout = next();
      if (layout !== 'legacy' && layout !== 'nested') {
        throw new Error(`Invalid --layout ${layout}; expected legacy or nested`);
      }
      options.layout = layout;
    } else if (arg === '--build-signature') {
      options.buildSignature = next();
    } else if (arg === '--no-copy') {
      options.copyFiles = false;
    } else {
      throw new Error(`Unknown argument ${arg}`);
    }
  }

  const targets =
    target === 'all' ? [...EXTENSION_BUILD_TARGETS] : [target];
  return { targets, options };
}

function parseTarget(value: string): ExtensionBuildTarget | 'all' {
  if (value === 'all') return value;
  if ((EXTENSION_BUILD_TARGETS as readonly string[]).includes(value)) {
    return value as ExtensionBuildTarget;
  }
  throw new Error(
    `Invalid --target ${value}; expected all, ${EXTENSION_BUILD_TARGETS.join(', ')}`
  );
}

if (import.meta.main) {
  main().catch(err => {
    console.error(err);
    process.exit(1);
  });
}
