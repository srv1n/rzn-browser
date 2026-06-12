import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  buildFor,
  buildMetadataForTarget,
  buildManifestForTarget,
  CHROMIUM_EXTENSION_TARGETS,
  chromeExtensionIdFromManifestKey,
  RZN_DEV_EXTENSION_ID,
  RZN_DEV_EXTENSION_ORIGIN,
  RZN_NATIVE_HOST_NAME,
  type ExtensionBuildTarget,
} from '../../scripts/build-ext';

const repoRoot = path.resolve(fileURLToPath(new URL('../../', import.meta.url)));

async function readJson(filePath: string) {
  return JSON.parse(await fs.readFile(filePath, 'utf8'));
}

describe('manifest generation', () => {
  it('generates Chrome, Edge, and Chromium MV3 manifests from base plus overlays', async () => {
    for (const target of CHROMIUM_EXTENSION_TARGETS) {
      const manifest = await buildManifestForTarget(target, repoRoot);

      expect(manifest.manifest_version).toBe(3);
      expect(manifest.permissions).toContain('nativeMessaging');
      expect(manifest.permissions).toContain('tabs');
      expect(manifest.host_permissions).toEqual(['<all_urls>']);
      expect(manifest.background).toEqual({ service_worker: 'background.js' });
      expect(manifest.content_scripts).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ js: ['contentScript.js'], run_at: 'document_idle' }),
        ])
      );
      expect(manifest.action).toMatchObject({
        default_popup: 'popup.html',
        default_icon: {
          '16': 'icons/brain-16.png',
          '128': 'icons/brain-128.png',
        },
      });
      expect(manifest.rzn_build).toBeUndefined();
      expect(buildMetadataForTarget(target)).toMatchObject({
        extension_target: target,
        browser_family: 'chromium',
        extension_id: RZN_DEV_EXTENSION_ID,
        extension_origin: RZN_DEV_EXTENSION_ORIGIN,
        native_host_name: RZN_NATIVE_HOST_NAME,
      });
    }
  });

  it('derives the pinned dev extension ID from the shared manifest key', async () => {
    const base = await readJson(path.join(repoRoot, 'extension', 'src', 'manifest.base.json'));

    expect(chromeExtensionIdFromManifestKey(base.key)).toBe(RZN_DEV_EXTENSION_ID);
  });

  it('keeps shared permissions and content scripts out of Chromium overlays', async () => {
    for (const target of CHROMIUM_EXTENSION_TARGETS) {
      const overlay = await readJson(
        path.join(repoRoot, 'extension', 'src', `manifest.${target}.json`)
      );

      expect(overlay.permissions).toBeUndefined();
      expect(overlay.host_permissions).toBeUndefined();
      expect(overlay.content_scripts).toBeUndefined();
      expect(overlay.background).toBeUndefined();
      expect(overlay.rzn_build).toBeUndefined();
    }
  });

  it('keeps Chromium manifests free of legacy Edge-only metadata', async () => {
    const chrome = await buildManifestForTarget('chrome', repoRoot);
    const edge = await buildManifestForTarget('edge', repoRoot);
    const chromium = await buildManifestForTarget('chromium', repoRoot);

    expect(edge.minimum_edge_version).toBeUndefined();
    expect(chrome.minimum_edge_version).toBeUndefined();
    expect(chromium.minimum_edge_version).toBeUndefined();
    expect(buildMetadataForTarget('chrome').extension_target).toBe('chrome');
  });

  it('writes generated target manifests and copies built assets', async () => {
    const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'rzn-manifest-generation-'));
    const distSourceDir = path.join(tmp, 'dist-source');
    const distRootDir = path.join(tmp, 'output');
    await fs.mkdir(path.join(distSourceDir, 'assets'), { recursive: true });
    await fs.writeFile(path.join(distSourceDir, 'background.js'), '// built background');
    await fs.writeFile(path.join(distSourceDir, 'assets', 'popup.js'), '// built popup');

    for (const target of CHROMIUM_EXTENSION_TARGETS) {
      await buildFor(target as ExtensionBuildTarget, {
        rootDir: repoRoot,
        distSourceDir,
        distRootDir,
        layout: 'nested',
      });

      const manifest = await readJson(path.join(distRootDir, target, 'manifest.json'));
      const metadata = await readJson(path.join(distRootDir, target, 'rzn-build.json'));
      expect(manifest.rzn_build).toBeUndefined();
      expect(metadata.extension_target).toBe(target);
      expect(metadata.extension_id).toBe(RZN_DEV_EXTENSION_ID);
      expect(metadata.extension_origin).toBe(RZN_DEV_EXTENSION_ORIGIN);
      expect(metadata.native_host_name).toBe(RZN_NATIVE_HOST_NAME);
      expect(metadata.build_signature).toEqual(expect.any(String));
      await expect(
        fs.readFile(path.join(distRootDir, target, 'background.js'), 'utf8')
      ).resolves.toBe('// built background');
      await expect(
        fs.readFile(path.join(distRootDir, target, 'assets', 'popup.js'), 'utf8')
      ).resolves.toBe('// built popup');
    }
  });

  it('keeps the legacy dist-target manifest layout available for older callers', async () => {
    const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'rzn-manifest-legacy-'));
    const distSourceDir = path.join(tmp, 'dist-source');
    await fs.mkdir(distSourceDir, { recursive: true });
    await fs.writeFile(path.join(distSourceDir, 'background.js'), '// built background');

    await buildFor('chrome', {
      rootDir: repoRoot,
      distSourceDir,
      distRootDir: tmp,
    });

    const manifest = await readJson(path.join(tmp, 'dist-chrome', 'manifest.json'));
    const metadata = await readJson(path.join(tmp, 'dist-chrome', 'rzn-build.json'));
    expect(manifest.rzn_build).toBeUndefined();
    expect(metadata.extension_target).toBe('chrome');
    expect(metadata.extension_id).toBe(RZN_DEV_EXTENSION_ID);
  });

  it('fails when a requested target overlay is missing', async () => {
    await expect(buildManifestForTarget('safari' as ExtensionBuildTarget, repoRoot)).rejects.toThrow(
      /manifest\.safari\.json/
    );
  });

  it('exposes CI-friendly build and package commands', async () => {
    const packageJson = await readJson(path.join(repoRoot, 'extension', 'package.json'));

    expect(packageJson.scripts['build']).toContain('./build.sh all');
    expect(packageJson.scripts['build:chrome']).toContain('./build.sh chrome');
    expect(packageJson.scripts['build:edge']).toContain('./build.sh edge');
    expect(packageJson.scripts['build:chromium']).toContain('./build.sh chromium');
    expect(packageJson.scripts['package:chrome']).toContain('tar -czf');
    expect(packageJson.scripts['package:edge']).toContain('tar -czf');
    expect(packageJson.scripts['package:chromium']).toContain('tar -czf');
  });
});
