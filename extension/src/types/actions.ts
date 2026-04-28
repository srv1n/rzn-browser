// Auto-generated from schema/actions-v1.json
// schema-version: rzn.actions.v1
// schema-sha256: 6873b747d9ea838443a4d94266ca07656567853722d8f89aba807a171bf469cf
// DO NOT EDIT MANUALLY

export interface RobustSelectors {
  primary?: string;
  fallbacks?: string[];
  confidence?: number;
  visualHash?: string;
}

export interface Selectors {
  css?: string;
  xpath?: string;
  text?: string;
  robust?: RobustSelectors;
}

export interface FieldSpec {
  name: string;
  selector: string;
  attribute?: string;
  post_processing?: string[];
  [key: string]: any;
}

export interface CookieSpec {
  name: string;
  value: string;
  domain?: string;
  path?: string;
  secure?: boolean;
  http_only?: boolean;
  expiration_date?: number;
  [key: string]: any;
}

export interface NavigateToUrl {
  type: 'navigate_to_url';
  url: string;
  wait?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface OpenNewTab {
  type: 'open_new_tab';
  url?: string;
  [key: string]: any;
}

export interface SwitchToTab {
  type: 'switch_to_tab';
  tab_identifier: any;
  [key: string]: any;
}

export interface CloseCurrentTab {
  type: 'close_current_tab';
  tab_identifier?: any;
  [key: string]: any;
}

export interface GetCurrentUrl {
  type: 'get_current_url';
  [key: string]: any;
}

export interface ClickElement {
  type: 'click_element';
  selector: string;
  frame_id?: string;
  random_offset?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface DblClickElement {
  type: 'dbl_click_element';
  selector: string;
  frame_id?: string;
  random_offset?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface HoverElement {
  type: 'hover_element';
  selector: string;
  frame_id?: string;
  random_offset?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface FillInputField {
  type: 'fill_input_field';
  selector: string;
  value: string;
  frame_id?: string;
  clear_first?: boolean;
  simulate_typing?: boolean;
  use_native_input?: boolean;
  typing_speed?: 'slow' | 'medium' | 'fast';
  delay_ms?: number;
  timeout_ms?: number;
  [key: string]: any;
}

export interface FillAndSubmit {
  type: 'fill_and_submit';
  selector: string;
  value: string;
  submit_selector?: string;
  submit_label_regex?: string;
  wait_for_increase_selector?: string;
  frame_id?: string;
  clear_first?: boolean;
  simulate_typing?: boolean;
  delay_ms?: number;
  timeout_ms?: number;
  wait_timeout_ms?: number;
  [key: string]: any;
}

export interface TypeText {
  type: 'type_text';
  selector: string;
  text?: string;
  value?: string;
  frame_id?: string;
  use_native_input?: boolean;
  delay_ms?: number;
  typing_speed?: 'slow' | 'medium' | 'fast';
  timeout_ms?: number;
  [key: string]: any;
}

export interface SubmitInput {
  type: 'submit_input';
  selector: string;
  text: string;
  frame_id?: string;
  clear_first?: boolean;
  simulate_typing?: boolean;
  delay_ms?: number;
  use_native_input?: boolean;
  submit_fallback?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface PressSpecialKey {
  type: 'press_special_key';
  key: string;
  selector?: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface SelectOptionInDropdown {
  type: 'select_option_in_dropdown';
  selector: string;
  value: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface UploadFile {
  type: 'upload_file';
  selector: string;
  file_path: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface DragAndDrop {
  type: 'drag_and_drop';
  source_selector: string;
  target_selector: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface ScrollWindowTo {
  type: 'scroll_window_to';
  direction?: string;
  amount?: number;
  delta?: number;
  x?: number;
  y?: number;
  wait_after_ms?: number;
  [key: string]: any;
}

export interface ScrollElementIntoView {
  type: 'scroll_element_into_view';
  selector: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface InfiniteScroll {
  type: 'infinite_scroll';
  max_scrolls?: number;
  scroll_delay?: number;
  target_selector?: string;
  target_count?: number;
  [key: string]: any;
}

export interface WaitForTimeout {
  type: 'wait_for_timeout';
  timeout_ms: number;
  [key: string]: any;
}

export interface WaitForElement {
  type: 'wait_for_element';
  selector: string;
  frame_id?: string;
  condition?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface WaitForNavigation {
  type: 'wait_for_navigation';
  url_pattern?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface WaitForNetworkIdle {
  type: 'wait_for_network_idle';
  idle_time_ms?: number;
  max_wait_ms?: number;
  [key: string]: any;
}

export interface ExtractStructuredData {
  type: 'extract_structured_data';
  item_selector: string;
  limit?: number;
  fields: any[];
  frame_id?: string;
  extraction_type?: string;
  [key: string]: any;
}

export interface GetElementText {
  type: 'get_element_text';
  selector: string;
  frame_id?: string;
  [key: string]: any;
}

export interface GetElementValue {
  type: 'get_element_value';
  selector: string;
  frame_id?: string;
  [key: string]: any;
}

export interface GetElementCount {
  type: 'get_element_count';
  selector: string;
  frame_id?: string;
  [key: string]: any;
}

export interface GetElementAttribute {
  type: 'get_element_attribute';
  selector: string;
  attribute: string;
  frame_id?: string;
  [key: string]: any;
}

export interface TakeScreenshot {
  type: 'take_screenshot';
  full_page?: boolean;
  annotate?: boolean;
  annotate_max_labels?: number;
  annotate_max_elements?: number;
  quality?: number;
  format?: string;
  [key: string]: any;
}

export interface GetPageSource {
  type: 'get_page_source';
  [key: string]: any;
}

export interface AssertSelectorState {
  type: 'assert_selector_state';
  selector: string;
  condition: string;
  frame_id?: string;
  [key: string]: any;
}

export interface AssertTextInElement {
  type: 'assert_text_in_element';
  selector: string;
  text: string;
  frame_id?: string;
  match_type?: string;
  [key: string]: any;
}

export interface AssertUrlMatches {
  type: 'assert_url_matches';
  url_pattern: string;
  match_type?: string;
  [key: string]: any;
}

export interface ExecuteJavascript {
  type: 'execute_javascript';
  script: string;
  args?: any[];
  return_value?: boolean;
  world?: 'isolated' | 'main';
  timeout_ms?: number;
  [key: string]: any;
}

export interface EvalMainWorld {
  type: 'eval_main_world';
  script: string;
  args?: any[];
  return_value?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface EvalIsolatedWorld {
  type: 'eval_isolated_world';
  script: string;
  args?: any[];
  return_value?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface InspectElement {
  type: 'inspect_element';
  selector: string;
  frame_id?: string;
  include_ancestors?: boolean;
  include_shadow_path?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface InspectClickSurface {
  type: 'inspect_click_surface';
  selector: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface CaptureUiBundle {
  type: 'capture_ui_bundle';
  selector?: string;
  include_dom_snapshot?: boolean;
  include_screenshot?: boolean;
  annotate?: boolean;
  max_elements?: number;
  timeout_ms?: number;
  [key: string]: any;
}

export interface VerifyUiChange {
  type: 'verify_ui_change';
  selector?: string;
  condition?: string;
  text?: string;
  match_type?: string;
  value_equals?: string;
  value_contains?: string;
  url_includes?: string;
  url_matches?: string;
  active_selector?: string;
  count_at_least?: number;
  count_equals?: number;
  all?: any[];
  any?: any[];
  timeout_ms?: number;
  [key: string]: any;
}

export interface ReadFieldValue {
  type: 'read_field_value';
  selector: string;
  frame_id?: string;
  timeout_ms?: number;
  [key: string]: any;
}

export interface SemanticAction {
  type: 'semantic_action';
  action: string;
  selector?: string;
  value?: string;
  key?: string;
  step?: any;
  postcondition?: any;
  postcondition_required?: boolean;
  timeout_ms?: number;
  [key: string]: any;
}

export interface SameOriginRequest {
  type: 'same_origin_request';
  method?: string;
  path: string;
  query?: any;
  headers?: any;
  body?: any;
  response_format?: 'json' | 'text';
  max_bytes?: number;
  [key: string]: any;
}

export interface SetCookie {
  type: 'set_cookie';
  cookie: any;
  [key: string]: any;
}

export interface GetCookies {
  type: 'get_cookies';
  domain?: string;
  [key: string]: any;
}

export interface ClearCookies {
  type: 'clear_cookies';
  domain?: string;
  [key: string]: any;
}

export interface SetLocalStorageItem {
  type: 'set_local_storage_item';
  storage_key: string;
  storage_value: string;
  [key: string]: any;
}

export interface GetLocalStorageItem {
  type: 'get_local_storage_item';
  storage_key: string;
  [key: string]: any;
}

export interface ClearLocalStorage {
  type: 'clear_local_storage';
  [key: string]: any;
}

export interface DownloadImages {
  type: 'download_images';
  selector: string;
  download_folder?: string;
  limit?: number;
  [key: string]: any;
}

export interface SimulateHumanBehavior {
  type: 'simulate_human_behavior';
  behaviors?: string[];
  [key: string]: any;
}

export interface DetectPopups {
  type: 'detect_popups';
  custom_selectors?: string[];
  [key: string]: any;
}

export interface DismissPopups {
  type: 'dismiss_popups';
  dismiss_selectors?: string[];
  [key: string]: any;
}

export interface WaitForNoPopups {
  type: 'wait_for_no_popups';
  timeout_ms?: number;
  [key: string]: any;
}

export interface HandleCaptcha {
  type: 'handle_captcha';
  [key: string]: any;
}

export interface ConfigureCaptchaSolver {
  type: 'configure_captcha_solver';
  solver?: string;
  api_key?: string;
  [key: string]: any;
}

export interface RequestUserIntervention {
  type: 'request_user_intervention';
  message?: string;
  instructions?: string;
  timeout_ms?: number;
  approval_mode?: 'ask_user' | 'notify' | 'auto_continue' | 'noop';
  approval_policy?: 'ask_user' | 'notify' | 'auto_continue' | 'noop';
  continue_on_timeout?: boolean;
  notification_title?: string;
  notification_message?: string;
  [key: string]: any;
}

export interface WaitForAuth {
  type: 'wait_for_auth';
  timeout_ms?: number;
  success_selectors?: string[];
  success_url_pattern?: string;
  [key: string]: any;
}

export interface WaitForTotp {
  type: 'wait_for_totp';
  timeout_ms?: number;
  totp_selectors?: string[];
  [key: string]: any;
}

export interface WaitForVerification {
  type: 'wait_for_verification';
  timeout_ms?: number;
  success_url_pattern?: string;
  success_selectors?: string[];
  [key: string]: any;
}

export interface ExtractPageAssets {
  type: 'extract_page_assets';
  asset_types?: string[];
  limit?: number;
  [key: string]: any;
}

export type Action = 
  | NavigateToUrl
  | OpenNewTab
  | SwitchToTab
  | CloseCurrentTab
  | GetCurrentUrl
  | ClickElement
  | DblClickElement
  | HoverElement
  | FillInputField
  | FillAndSubmit
  | TypeText
  | SubmitInput
  | PressSpecialKey
  | SelectOptionInDropdown
  | UploadFile
  | DragAndDrop
  | ScrollWindowTo
  | ScrollElementIntoView
  | InfiniteScroll
  | WaitForTimeout
  | WaitForElement
  | WaitForNavigation
  | WaitForNetworkIdle
  | ExtractStructuredData
  | GetElementText
  | GetElementValue
  | GetElementCount
  | GetElementAttribute
  | TakeScreenshot
  | GetPageSource
  | AssertSelectorState
  | AssertTextInElement
  | AssertUrlMatches
  | ExecuteJavascript
  | EvalMainWorld
  | EvalIsolatedWorld
  | InspectElement
  | InspectClickSurface
  | CaptureUiBundle
  | VerifyUiChange
  | ReadFieldValue
  | SemanticAction
  | SameOriginRequest
  | SetCookie
  | GetCookies
  | ClearCookies
  | SetLocalStorageItem
  | GetLocalStorageItem
  | ClearLocalStorage
  | DownloadImages
  | SimulateHumanBehavior
  | DetectPopups
  | DismissPopups
  | WaitForNoPopups
  | HandleCaptcha
  | ConfigureCaptchaSolver
  | RequestUserIntervention
  | WaitForAuth
  | WaitForTotp
  | WaitForVerification
  | ExtractPageAssets;

// Type guards
export function isNavigateToUrl(action: Action): action is NavigateToUrl {
  return action.type === 'navigate_to_url';
}

export function isOpenNewTab(action: Action): action is OpenNewTab {
  return action.type === 'open_new_tab';
}

export function isSwitchToTab(action: Action): action is SwitchToTab {
  return action.type === 'switch_to_tab';
}

export function isCloseCurrentTab(action: Action): action is CloseCurrentTab {
  return action.type === 'close_current_tab';
}

export function isGetCurrentUrl(action: Action): action is GetCurrentUrl {
  return action.type === 'get_current_url';
}

export function isClickElement(action: Action): action is ClickElement {
  return action.type === 'click_element';
}

export function isDblClickElement(action: Action): action is DblClickElement {
  return action.type === 'dbl_click_element';
}

export function isHoverElement(action: Action): action is HoverElement {
  return action.type === 'hover_element';
}

export function isFillInputField(action: Action): action is FillInputField {
  return action.type === 'fill_input_field';
}

export function isFillAndSubmit(action: Action): action is FillAndSubmit {
  return action.type === 'fill_and_submit';
}

export function isTypeText(action: Action): action is TypeText {
  return action.type === 'type_text';
}

export function isSubmitInput(action: Action): action is SubmitInput {
  return action.type === 'submit_input';
}

export function isPressSpecialKey(action: Action): action is PressSpecialKey {
  return action.type === 'press_special_key';
}

export function isSelectOptionInDropdown(action: Action): action is SelectOptionInDropdown {
  return action.type === 'select_option_in_dropdown';
}

export function isUploadFile(action: Action): action is UploadFile {
  return action.type === 'upload_file';
}

export function isDragAndDrop(action: Action): action is DragAndDrop {
  return action.type === 'drag_and_drop';
}

export function isScrollWindowTo(action: Action): action is ScrollWindowTo {
  return action.type === 'scroll_window_to';
}

export function isScrollElementIntoView(action: Action): action is ScrollElementIntoView {
  return action.type === 'scroll_element_into_view';
}

export function isInfiniteScroll(action: Action): action is InfiniteScroll {
  return action.type === 'infinite_scroll';
}

export function isWaitForTimeout(action: Action): action is WaitForTimeout {
  return action.type === 'wait_for_timeout';
}

export function isWaitForElement(action: Action): action is WaitForElement {
  return action.type === 'wait_for_element';
}

export function isWaitForNavigation(action: Action): action is WaitForNavigation {
  return action.type === 'wait_for_navigation';
}

export function isWaitForNetworkIdle(action: Action): action is WaitForNetworkIdle {
  return action.type === 'wait_for_network_idle';
}

export function isExtractStructuredData(action: Action): action is ExtractStructuredData {
  return action.type === 'extract_structured_data';
}

export function isGetElementText(action: Action): action is GetElementText {
  return action.type === 'get_element_text';
}

export function isGetElementValue(action: Action): action is GetElementValue {
  return action.type === 'get_element_value';
}

export function isGetElementCount(action: Action): action is GetElementCount {
  return action.type === 'get_element_count';
}

export function isGetElementAttribute(action: Action): action is GetElementAttribute {
  return action.type === 'get_element_attribute';
}

export function isTakeScreenshot(action: Action): action is TakeScreenshot {
  return action.type === 'take_screenshot';
}

export function isGetPageSource(action: Action): action is GetPageSource {
  return action.type === 'get_page_source';
}

export function isAssertSelectorState(action: Action): action is AssertSelectorState {
  return action.type === 'assert_selector_state';
}

export function isAssertTextInElement(action: Action): action is AssertTextInElement {
  return action.type === 'assert_text_in_element';
}

export function isAssertUrlMatches(action: Action): action is AssertUrlMatches {
  return action.type === 'assert_url_matches';
}

export function isExecuteJavascript(action: Action): action is ExecuteJavascript {
  return action.type === 'execute_javascript';
}

export function isEvalMainWorld(action: Action): action is EvalMainWorld {
  return action.type === 'eval_main_world';
}

export function isEvalIsolatedWorld(action: Action): action is EvalIsolatedWorld {
  return action.type === 'eval_isolated_world';
}

export function isInspectElement(action: Action): action is InspectElement {
  return action.type === 'inspect_element';
}

export function isInspectClickSurface(action: Action): action is InspectClickSurface {
  return action.type === 'inspect_click_surface';
}

export function isCaptureUiBundle(action: Action): action is CaptureUiBundle {
  return action.type === 'capture_ui_bundle';
}

export function isVerifyUiChange(action: Action): action is VerifyUiChange {
  return action.type === 'verify_ui_change';
}

export function isReadFieldValue(action: Action): action is ReadFieldValue {
  return action.type === 'read_field_value';
}

export function isSemanticAction(action: Action): action is SemanticAction {
  return action.type === 'semantic_action';
}

export function isSameOriginRequest(action: Action): action is SameOriginRequest {
  return action.type === 'same_origin_request';
}

export function isSetCookie(action: Action): action is SetCookie {
  return action.type === 'set_cookie';
}

export function isGetCookies(action: Action): action is GetCookies {
  return action.type === 'get_cookies';
}

export function isClearCookies(action: Action): action is ClearCookies {
  return action.type === 'clear_cookies';
}

export function isSetLocalStorageItem(action: Action): action is SetLocalStorageItem {
  return action.type === 'set_local_storage_item';
}

export function isGetLocalStorageItem(action: Action): action is GetLocalStorageItem {
  return action.type === 'get_local_storage_item';
}

export function isClearLocalStorage(action: Action): action is ClearLocalStorage {
  return action.type === 'clear_local_storage';
}

export function isDownloadImages(action: Action): action is DownloadImages {
  return action.type === 'download_images';
}

export function isSimulateHumanBehavior(action: Action): action is SimulateHumanBehavior {
  return action.type === 'simulate_human_behavior';
}

export function isDetectPopups(action: Action): action is DetectPopups {
  return action.type === 'detect_popups';
}

export function isDismissPopups(action: Action): action is DismissPopups {
  return action.type === 'dismiss_popups';
}

export function isWaitForNoPopups(action: Action): action is WaitForNoPopups {
  return action.type === 'wait_for_no_popups';
}

export function isHandleCaptcha(action: Action): action is HandleCaptcha {
  return action.type === 'handle_captcha';
}

export function isConfigureCaptchaSolver(action: Action): action is ConfigureCaptchaSolver {
  return action.type === 'configure_captcha_solver';
}

export function isRequestUserIntervention(action: Action): action is RequestUserIntervention {
  return action.type === 'request_user_intervention';
}

export function isWaitForAuth(action: Action): action is WaitForAuth {
  return action.type === 'wait_for_auth';
}

export function isWaitForTotp(action: Action): action is WaitForTotp {
  return action.type === 'wait_for_totp';
}

export function isWaitForVerification(action: Action): action is WaitForVerification {
  return action.type === 'wait_for_verification';
}

export function isExtractPageAssets(action: Action): action is ExtractPageAssets {
  return action.type === 'extract_page_assets';
}


// Zod schemas for runtime validation
import { z } from 'zod';

export const RobustSelectorsSchema = z.object({
  primary: z.string().optional(),
  fallbacks: z.array(z.string()).optional(),
  confidence: z.number().min(0).max(1).optional(),
  visualHash: z.string().regex(/^[A-Fa-f0-9]{64}$/).optional(),
});

export const SelectorsSchema = z.object({
  css: z.string().optional(),
  xpath: z.string().optional(),
  text: z.string().optional(),
  robust: RobustSelectorsSchema.optional(),
});

export const FieldSpecSchema = z.object({
  name: z.string(),
  selector: z.string(),
  attribute: z.string().optional(),
  post_processing: z.array(z.string()).optional(),
}).passthrough();

export const CookieSpecSchema = z.object({
  name: z.string(),
  value: z.string(),
  domain: z.string().optional(),
  path: z.string().optional(),
  secure: z.boolean().optional(),
  http_only: z.boolean().optional(),
  expiration_date: z.number().optional(),
}).passthrough();

export const NavigateToUrlSchema = z.object({
  type: z.literal('navigate_to_url'),
  url: z.string(),
  wait: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const OpenNewTabSchema = z.object({
  type: z.literal('open_new_tab'),
  url: z.string().optional(),
}).passthrough();

export const SwitchToTabSchema = z.object({
  type: z.literal('switch_to_tab'),
  tab_identifier: z.any(),
}).passthrough();

export const CloseCurrentTabSchema = z.object({
  type: z.literal('close_current_tab'),
  tab_identifier: z.any().optional(),
}).passthrough();

export const GetCurrentUrlSchema = z.object({
  type: z.literal('get_current_url'),
}).passthrough();

export const ClickElementSchema = z.object({
  type: z.literal('click_element'),
  selector: z.string(),
  frame_id: z.string().optional(),
  random_offset: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const DblClickElementSchema = z.object({
  type: z.literal('dbl_click_element'),
  selector: z.string(),
  frame_id: z.string().optional(),
  random_offset: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const HoverElementSchema = z.object({
  type: z.literal('hover_element'),
  selector: z.string(),
  frame_id: z.string().optional(),
  random_offset: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const FillInputFieldSchema = z.object({
  type: z.literal('fill_input_field'),
  selector: z.string(),
  value: z.string(),
  frame_id: z.string().optional(),
  clear_first: z.boolean().optional(),
  simulate_typing: z.boolean().optional(),
  use_native_input: z.boolean().optional(),
  typing_speed: z.enum(['slow', 'medium', 'fast']).optional(),
  delay_ms: z.number().int().min(0).optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const FillAndSubmitSchema = z.object({
  type: z.literal('fill_and_submit'),
  selector: z.string(),
  value: z.string(),
  submit_selector: z.string().optional(),
  submit_label_regex: z.string().optional(),
  wait_for_increase_selector: z.string().optional(),
  frame_id: z.string().optional(),
  clear_first: z.boolean().optional(),
  simulate_typing: z.boolean().optional(),
  delay_ms: z.number().int().min(0).optional(),
  timeout_ms: z.number().int().min(0).optional(),
  wait_timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const TypeTextSchema = z.object({
  type: z.literal('type_text'),
  selector: z.string(),
  text: z.string().optional(),
  value: z.string().optional(),
  frame_id: z.string().optional(),
  use_native_input: z.boolean().optional(),
  delay_ms: z.number().int().min(0).optional(),
  typing_speed: z.enum(['slow', 'medium', 'fast']).optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const SubmitInputSchema = z.object({
  type: z.literal('submit_input'),
  selector: z.string(),
  text: z.string(),
  frame_id: z.string().optional(),
  clear_first: z.boolean().optional(),
  simulate_typing: z.boolean().optional(),
  delay_ms: z.number().int().min(0).optional(),
  use_native_input: z.boolean().optional(),
  submit_fallback: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const PressSpecialKeySchema = z.object({
  type: z.literal('press_special_key'),
  key: z.string(),
  selector: z.string().optional(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const SelectOptionInDropdownSchema = z.object({
  type: z.literal('select_option_in_dropdown'),
  selector: z.string(),
  value: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const UploadFileSchema = z.object({
  type: z.literal('upload_file'),
  selector: z.string(),
  file_path: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const DragAndDropSchema = z.object({
  type: z.literal('drag_and_drop'),
  source_selector: z.string(),
  target_selector: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const ScrollWindowToSchema = z.object({
  type: z.literal('scroll_window_to'),
  direction: z.string().optional(),
  amount: z.number().int().optional(),
  delta: z.number().int().optional(),
  x: z.number().int().optional(),
  y: z.number().int().optional(),
  wait_after_ms: z.number().int().min(0).optional(),
}).passthrough();

export const ScrollElementIntoViewSchema = z.object({
  type: z.literal('scroll_element_into_view'),
  selector: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const InfiniteScrollSchema = z.object({
  type: z.literal('infinite_scroll'),
  max_scrolls: z.number().int().min(0).optional(),
  scroll_delay: z.number().int().min(0).optional(),
  target_selector: z.string().optional(),
  target_count: z.number().int().min(0).optional(),
}).passthrough();

export const WaitForTimeoutSchema = z.object({
  type: z.literal('wait_for_timeout'),
  timeout_ms: z.number().int().min(0),
}).passthrough();

export const WaitForElementSchema = z.object({
  type: z.literal('wait_for_element'),
  selector: z.string(),
  frame_id: z.string().optional(),
  condition: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const WaitForNavigationSchema = z.object({
  type: z.literal('wait_for_navigation'),
  url_pattern: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const WaitForNetworkIdleSchema = z.object({
  type: z.literal('wait_for_network_idle'),
  idle_time_ms: z.number().int().min(0).optional(),
  max_wait_ms: z.number().int().min(0).optional(),
}).passthrough();

export const ExtractStructuredDataSchema = z.object({
  type: z.literal('extract_structured_data'),
  item_selector: z.string(),
  limit: z.number().int().min(1).optional(),
  fields: z.array(z.any()),
  frame_id: z.string().optional(),
  extraction_type: z.string().optional(),
}).passthrough();

export const GetElementTextSchema = z.object({
  type: z.literal('get_element_text'),
  selector: z.string(),
  frame_id: z.string().optional(),
}).passthrough();

export const GetElementValueSchema = z.object({
  type: z.literal('get_element_value'),
  selector: z.string(),
  frame_id: z.string().optional(),
}).passthrough();

export const GetElementCountSchema = z.object({
  type: z.literal('get_element_count'),
  selector: z.string(),
  frame_id: z.string().optional(),
}).passthrough();

export const GetElementAttributeSchema = z.object({
  type: z.literal('get_element_attribute'),
  selector: z.string(),
  attribute: z.string(),
  frame_id: z.string().optional(),
}).passthrough();

export const TakeScreenshotSchema = z.object({
  type: z.literal('take_screenshot'),
  full_page: z.boolean().optional(),
  annotate: z.boolean().optional(),
  annotate_max_labels: z.number().int().min(1).max(200).optional(),
  annotate_max_elements: z.number().int().min(1).max(200).optional(),
  quality: z.number().int().min(0).max(100).optional(),
  format: z.string().optional(),
}).passthrough();

export const GetPageSourceSchema = z.object({
  type: z.literal('get_page_source'),
}).passthrough();

export const AssertSelectorStateSchema = z.object({
  type: z.literal('assert_selector_state'),
  selector: z.string(),
  condition: z.string(),
  frame_id: z.string().optional(),
}).passthrough();

export const AssertTextInElementSchema = z.object({
  type: z.literal('assert_text_in_element'),
  selector: z.string(),
  text: z.string(),
  frame_id: z.string().optional(),
  match_type: z.string().optional(),
}).passthrough();

export const AssertUrlMatchesSchema = z.object({
  type: z.literal('assert_url_matches'),
  url_pattern: z.string(),
  match_type: z.string().optional(),
}).passthrough();

export const ExecuteJavascriptSchema = z.object({
  type: z.literal('execute_javascript'),
  script: z.string(),
  args: z.array(z.any()).optional(),
  return_value: z.boolean().optional(),
  world: z.enum(['isolated', 'main']).optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const EvalMainWorldSchema = z.object({
  type: z.literal('eval_main_world'),
  script: z.string(),
  args: z.array(z.any()).optional(),
  return_value: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const EvalIsolatedWorldSchema = z.object({
  type: z.literal('eval_isolated_world'),
  script: z.string(),
  args: z.array(z.any()).optional(),
  return_value: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const InspectElementSchema = z.object({
  type: z.literal('inspect_element'),
  selector: z.string(),
  frame_id: z.string().optional(),
  include_ancestors: z.boolean().optional(),
  include_shadow_path: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const InspectClickSurfaceSchema = z.object({
  type: z.literal('inspect_click_surface'),
  selector: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const CaptureUiBundleSchema = z.object({
  type: z.literal('capture_ui_bundle'),
  selector: z.string().optional(),
  include_dom_snapshot: z.boolean().optional(),
  include_screenshot: z.boolean().optional(),
  annotate: z.boolean().optional(),
  max_elements: z.number().int().min(1).optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const VerifyUiChangeSchema = z.object({
  type: z.literal('verify_ui_change'),
  selector: z.string().optional(),
  condition: z.string().optional(),
  text: z.string().optional(),
  match_type: z.string().optional(),
  value_equals: z.string().optional(),
  value_contains: z.string().optional(),
  url_includes: z.string().optional(),
  url_matches: z.string().optional(),
  active_selector: z.string().optional(),
  count_at_least: z.number().int().min(0).optional(),
  count_equals: z.number().int().min(0).optional(),
  all: z.array(z.any()).optional(),
  any: z.array(z.any()).optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const ReadFieldValueSchema = z.object({
  type: z.literal('read_field_value'),
  selector: z.string(),
  frame_id: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const SemanticActionSchema = z.object({
  type: z.literal('semantic_action'),
  action: z.string(),
  selector: z.string().optional(),
  value: z.string().optional(),
  key: z.string().optional(),
  step: z.any().optional(),
  postcondition: z.any().optional(),
  postcondition_required: z.boolean().optional(),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const SameOriginRequestSchema = z.object({
  type: z.literal('same_origin_request'),
  method: z.string().optional(),
  path: z.string(),
  query: z.any().optional(),
  headers: z.any().optional(),
  body: z.any().optional(),
  response_format: z.enum(['json', 'text']).optional(),
  max_bytes: z.number().int().optional(),
}).passthrough();

export const SetCookieSchema = z.object({
  type: z.literal('set_cookie'),
  cookie: z.any(),
}).passthrough();

export const GetCookiesSchema = z.object({
  type: z.literal('get_cookies'),
  domain: z.string().optional(),
}).passthrough();

export const ClearCookiesSchema = z.object({
  type: z.literal('clear_cookies'),
  domain: z.string().optional(),
}).passthrough();

export const SetLocalStorageItemSchema = z.object({
  type: z.literal('set_local_storage_item'),
  storage_key: z.string(),
  storage_value: z.string(),
}).passthrough();

export const GetLocalStorageItemSchema = z.object({
  type: z.literal('get_local_storage_item'),
  storage_key: z.string(),
}).passthrough();

export const ClearLocalStorageSchema = z.object({
  type: z.literal('clear_local_storage'),
}).passthrough();

export const DownloadImagesSchema = z.object({
  type: z.literal('download_images'),
  selector: z.string(),
  download_folder: z.string().optional(),
  limit: z.number().int().min(0).optional(),
}).passthrough();

export const SimulateHumanBehaviorSchema = z.object({
  type: z.literal('simulate_human_behavior'),
  behaviors: z.array(z.string()).optional(),
}).passthrough();

export const DetectPopupsSchema = z.object({
  type: z.literal('detect_popups'),
  custom_selectors: z.array(z.string()).optional(),
}).passthrough();

export const DismissPopupsSchema = z.object({
  type: z.literal('dismiss_popups'),
  dismiss_selectors: z.array(z.string()).optional(),
}).passthrough();

export const WaitForNoPopupsSchema = z.object({
  type: z.literal('wait_for_no_popups'),
  timeout_ms: z.number().int().min(0).optional(),
}).passthrough();

export const HandleCaptchaSchema = z.object({
  type: z.literal('handle_captcha'),
}).passthrough();

export const ConfigureCaptchaSolverSchema = z.object({
  type: z.literal('configure_captcha_solver'),
  solver: z.string().optional(),
  api_key: z.string().optional(),
}).passthrough();

export const RequestUserInterventionSchema = z.object({
  type: z.literal('request_user_intervention'),
  message: z.string().optional(),
  instructions: z.string().optional(),
  timeout_ms: z.number().int().min(0).optional(),
  approval_mode: z.enum(['ask_user', 'notify', 'auto_continue', 'noop']).optional(),
  approval_policy: z.enum(['ask_user', 'notify', 'auto_continue', 'noop']).optional(),
  continue_on_timeout: z.boolean().optional(),
  notification_title: z.string().optional(),
  notification_message: z.string().optional(),
}).passthrough();

export const WaitForAuthSchema = z.object({
  type: z.literal('wait_for_auth'),
  timeout_ms: z.number().int().min(0).optional(),
  success_selectors: z.array(z.string()).optional(),
  success_url_pattern: z.string().optional(),
}).passthrough();

export const WaitForTotpSchema = z.object({
  type: z.literal('wait_for_totp'),
  timeout_ms: z.number().int().min(0).optional(),
  totp_selectors: z.array(z.string()).optional(),
}).passthrough();

export const WaitForVerificationSchema = z.object({
  type: z.literal('wait_for_verification'),
  timeout_ms: z.number().int().min(0).optional(),
  success_url_pattern: z.string().optional(),
  success_selectors: z.array(z.string()).optional(),
}).passthrough();

export const ExtractPageAssetsSchema = z.object({
  type: z.literal('extract_page_assets'),
  asset_types: z.array(z.string()).optional(),
  limit: z.number().int().min(0).optional(),
}).passthrough();

export const ActionSchema = z.discriminatedUnion('type', [
  NavigateToUrlSchema,
  OpenNewTabSchema,
  SwitchToTabSchema,
  CloseCurrentTabSchema,
  GetCurrentUrlSchema,
  ClickElementSchema,
  DblClickElementSchema,
  HoverElementSchema,
  FillInputFieldSchema,
  FillAndSubmitSchema,
  TypeTextSchema,
  SubmitInputSchema,
  PressSpecialKeySchema,
  SelectOptionInDropdownSchema,
  UploadFileSchema,
  DragAndDropSchema,
  ScrollWindowToSchema,
  ScrollElementIntoViewSchema,
  InfiniteScrollSchema,
  WaitForTimeoutSchema,
  WaitForElementSchema,
  WaitForNavigationSchema,
  WaitForNetworkIdleSchema,
  ExtractStructuredDataSchema,
  GetElementTextSchema,
  GetElementValueSchema,
  GetElementCountSchema,
  GetElementAttributeSchema,
  TakeScreenshotSchema,
  GetPageSourceSchema,
  AssertSelectorStateSchema,
  AssertTextInElementSchema,
  AssertUrlMatchesSchema,
  ExecuteJavascriptSchema,
  EvalMainWorldSchema,
  EvalIsolatedWorldSchema,
  InspectElementSchema,
  InspectClickSurfaceSchema,
  CaptureUiBundleSchema,
  VerifyUiChangeSchema,
  ReadFieldValueSchema,
  SemanticActionSchema,
  SameOriginRequestSchema,
  SetCookieSchema,
  GetCookiesSchema,
  ClearCookiesSchema,
  SetLocalStorageItemSchema,
  GetLocalStorageItemSchema,
  ClearLocalStorageSchema,
  DownloadImagesSchema,
  SimulateHumanBehaviorSchema,
  DetectPopupsSchema,
  DismissPopupsSchema,
  WaitForNoPopupsSchema,
  HandleCaptchaSchema,
  ConfigureCaptchaSolverSchema,
  RequestUserInterventionSchema,
  WaitForAuthSchema,
  WaitForTotpSchema,
  WaitForVerificationSchema,
  ExtractPageAssetsSchema,
]);

export type ActionFromSchema = z.infer<typeof ActionSchema>;
