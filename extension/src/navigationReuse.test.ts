import { describe, expect, it } from 'vitest';
import {
  firstWorkflowStepNeedsInitializedTab,
  navigationReuseDisposition,
  shouldSkipWorkflowNavigation,
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
