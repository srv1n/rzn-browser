import { describe, expect, it } from 'vitest';
import {
  canNavigateInPage,
  firstWorkflowStepNeedsInitializedTab,
  navigationReuseDisposition,
  shouldSkipWorkflowNavigation,
  siteChallengeKind,
  type PageFacts,
} from './navigationReuse';

describe('firstWorkflowStepNeedsInitializedTab', () => {
  it('lets a first navigation create exactly its destination tab', () => {
    expect(firstWorkflowStepNeedsInitializedTab('navigate_to_url')).toBe(false);
  });

  it('does not stage a tab for actions that create or close one themselves', () => {
    expect(firstWorkflowStepNeedsInitializedTab('open_new_tab')).toBe(false);
    expect(firstWorkflowStepNeedsInitializedTab('close_current_tab')).toBe(false);
  });

  it('initializes a tab only for a first action that needs page state', () => {
    expect(firstWorkflowStepNeedsInitializedTab('execute_javascript')).toBe(true);
  });
});

describe('shouldSkipWorkflowNavigation', () => {
  it('reuses direct and Project routes for the same conversation', () => {
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-1', '/c/chat-1')).toBe(true);
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/g/g-p-demo/c/chat-1', '/c/chat-1')).toBe(true);
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-1?model=gpt-5', '/c/chat-1')).toBe(true);
  });

  it('does not reuse another conversation, a matching prefix, or an empty matcher', () => {
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-2', '/c/chat-1')).toBe(false);
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-10', '/c/chat-1')).toBe(false);
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-1', '')).toBe(false);
    expect(shouldSkipWorkflowNavigation('https://chatgpt.com/c/chat-1', undefined)).toBe(false);
  });

  it('wakes a discarded matching tab instead of treating its retained URL as ready', () => {
    expect(navigationReuseDisposition('https://chatgpt.com/c/chat-1', '/c/chat-1', true))
      .toBe('wake_and_skip');
    expect(navigationReuseDisposition('https://chatgpt.com/c/chat-1', '/c/chat-1', false))
      .toBe('skip');
    expect(navigationReuseDisposition('https://chatgpt.com/c/chat-2', '/c/chat-1', true))
      .toBe('navigate');
  });
});

describe('canNavigateInPage', () => {
  it('stays in the page for a loaded tab on the same site', () => {
    expect(canNavigateInPage('https://www.g2.com/search?q=a', 'https://www.g2.com/products/x', 'complete')).toBe(true);
    expect(canNavigateInPage('https://old.reddit.com/r/a', 'https://www.reddit.com/r/b', 'complete')).toBe(true);
  });

  it('uses a browser navigation across sites, while loading, or from a blank tab', () => {
    expect(canNavigateInPage('https://www.g2.com/', 'https://www.capterra.com/', 'complete')).toBe(false);
    expect(canNavigateInPage('https://www.g2.com/', 'https://www.g2.com/a', 'loading')).toBe(false);
    expect(canNavigateInPage('about:blank', 'https://www.g2.com/a', 'complete')).toBe(false);
    expect(canNavigateInPage('chrome://newtab/', 'https://www.g2.com/a', 'complete')).toBe(false);
  });

  it('does not navigate in the page for a fragment-only change', () => {
    expect(canNavigateInPage('https://a.com/p', 'https://a.com/p#reviews', 'complete')).toBe(false);
    expect(canNavigateInPage('https://a.com/p#x', 'https://a.com/q#reviews', 'complete')).toBe(true);
  });
});

describe('siteChallengeKind', () => {
  const page = (over: Partial<PageFacts>): PageFacts => ({
    url: 'https://www.example.com/',
    title: 'Example',
    selectorHits: [],
    bodyText: 'hello',
    bodyLength: 5,
    ...over,
  });

  it('names the bot check from challenge-only markers', () => {
    expect(siteChallengeKind(page({ title: 'Just a moment...' }))).toBe('challenge');
    expect(siteChallengeKind(page({ selectorHits: ["iframe[src*='captcha-delivery.com']"] }))).toBe('datadome');
    expect(siteChallengeKind(page({ selectorHits: ['#challenge-form'] }))).toBe('cloudflare');
    expect(siteChallengeKind(page({ url: 'https://www.google.com/sorry/index?continue=x' }))).toBe('google');
    expect(siteChallengeKind(page({ url: 'https://www.amazon.com/errors/validateCaptcha' }))).toBe('amazon');
    expect(siteChallengeKind(page({ bodyText: 'Slide right to secure your access' }))).toBe('challenge');
    expect(siteChallengeKind(page({ title: 'Access Denied', bodyText: 'Reference #18.1' }))).toBe('blocked');
  });

  it('ignores normal pages, including long pages that mention the phrases', () => {
    expect(siteChallengeKind(page({}))).toBeNull();
    expect(siteChallengeKind(page({ url: 'https://www.example.com/sorry/' }))).toBeNull();
    const article = 'are you a robot? '.repeat(400);
    expect(siteChallengeKind(page({ bodyText: article.slice(0, 4000), bodyLength: article.length }))).toBeNull();
  });
});
