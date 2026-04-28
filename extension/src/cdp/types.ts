// CDP Type Definitions - Complete type safety for Chrome DevTools Protocol

// === Target Domain Types ===

export interface TargetInfo {
  targetId: string;
  type: 'page' | 'background_page' | 'service_worker' | 'shared_worker' | 'other' | 'browser' | 'iframe';
  title: string;
  url: string;
  attached: boolean;
  canAccessOpener: boolean;
  openerFrameId?: string;
  browserContextId?: string;
  openerId?: string;
}

export interface SessionInfo {
  sessionId: string;
}

// === Page Domain Types ===

export interface Frame {
  id: string;
  parentId?: string;
  loaderId: string;
  name?: string;
  url: string;
  urlFragment?: string;
  domainAndRegistry: string;
  securityOrigin: string;
  mimeType: string;
  unreachableUrl?: string;
  adFrameStatus?: {
    adFrameType: 'none' | 'child' | 'root';
    explanations?: string[];
  };
  secureContextType: 'Secure' | 'SecureLocalhost' | 'InsecureScheme' | 'InsecureAncestor';
  crossOriginIsolatedContextType: 'Isolated' | 'NotIsolated' | 'NotIsolatedFeatureDisabled';
  gatedAPIFeatures?: string[];
}

export interface FrameTree {
  frame: Frame;
  childFrames?: FrameTree[];
  resources?: FrameResource[];
}

export interface FrameResource {
  url: string;
  type: ResourceType;
  mimeType: string;
  lastModified?: number;
  contentSize?: number;
  failed?: boolean;
  canceled?: boolean;
}

export type ResourceType = 
  | 'Document'
  | 'Stylesheet' 
  | 'Image'
  | 'Media'
  | 'Font'
  | 'Script'
  | 'TextTrack'
  | 'XHR'
  | 'Fetch'
  | 'Prefetch'
  | 'EventSource'
  | 'WebSocket'
  | 'Manifest'
  | 'SignedExchange'
  | 'Ping'
  | 'CSPViolationReport'
  | 'Preflight'
  | 'Other';

export interface LayoutMetrics {
  layoutViewport: LayoutViewport;
  visualViewport: VisualViewport;
  contentSize: DOM.Rect;
  cssLayoutViewport?: LayoutViewport;
  cssVisualViewport?: VisualViewport;
  cssContentSize?: DOM.Rect;
}

export interface LayoutViewport {
  pageX: number;
  pageY: number;
  clientWidth: number;
  clientHeight: number;
}

export interface VisualViewport {
  offsetX: number;
  offsetY: number;
  pageX: number;
  pageY: number;
  clientWidth: number;
  clientHeight: number;
  scale: number;
  zoom?: number;
}

export interface Viewport {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

// === DOM Domain Types ===

export namespace DOM {
  export type NodeId = number;
  export type BackendNodeId = number;

  export interface Node {
    nodeId: NodeId;
    parentId?: NodeId;
    backendNodeId: BackendNodeId;
    nodeType: number;
    nodeName: string;
    localName: string;
    nodeValue: string;
    childNodeCount?: number;
    children?: Node[];
    attributes?: string[];
    documentURL?: string;
    baseURL?: string;
    publicId?: string;
    systemId?: string;
    internalSubset?: string;
    xmlVersion?: string;
    name?: string;
    value?: string;
    pseudoType?: PseudoType;
    pseudoIdentifier?: string;
    shadowRootType?: ShadowRootType;
    frameId?: string;
    contentDocument?: Node;
    shadowRoots?: Node[];
    templateContent?: Node;
    pseudoElements?: Node[];
    importedDocument?: Node;
    distributedNodes?: BackendNode[];
    isSVG?: boolean;
    compatibilityMode?: CompatibilityMode;
    assignedSlot?: BackendNode;
  }

  export interface BackendNode {
    nodeType: number;
    nodeName: string;
    backendNodeId: BackendNodeId;
  }

  export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
  }

  export interface BoxModel {
    content: number[];
    padding: number[];
    border: number[];
    margin: number[];
    width: number;
    height: number;
    shapeOutside?: ShapeOutsideInfo;
  }

  export interface ShapeOutsideInfo {
    bounds: number[];
    shape: any[];
    marginShape: any[];
  }

  export type PseudoType =
    | 'first-line'
    | 'first-letter'
    | 'before'
    | 'after'
    | 'marker'
    | 'backdrop'
    | 'selection'
    | 'search-text'
    | 'target-text'
    | 'spelling-error'
    | 'grammar-error'
    | 'highlight'
    | 'first-line-inherited'
    | 'scrollbar'
    | 'scrollbar-thumb'
    | 'scrollbar-button'
    | 'scrollbar-track'
    | 'scrollbar-track-piece'
    | 'scrollbar-corner'
    | 'resizer'
    | 'input-list-button'
    | 'view-transition'
    | 'view-transition-group'
    | 'view-transition-image-pair'
    | 'view-transition-old'
    | 'view-transition-new';

  export type ShadowRootType = 'user-agent' | 'open' | 'closed';

  export type CompatibilityMode = 'QuirksMode' | 'LimitedQuirksMode' | 'NoQuirksMode';

  export interface RGBA {
    r: number;
    g: number;
    b: number;
    a?: number;
  }
}

// === Runtime Domain Types ===

export namespace Runtime {
  export interface ExecutionContext {
    id: number;
    origin: string;
    name: string;
    uniqueId: string;
    auxData?: any;
  }

  export interface RemoteObject {
    type: ObjectType;
    subtype?: ObjectSubtype;
    className?: string;
    value?: any;
    unserializableValue?: UnserializableValue;
    description?: string;
    deepSerializedValue?: DeepSerializedValue;
    objectId?: RemoteObjectId;
    preview?: ObjectPreview;
    customPreview?: CustomPreview;
  }

  export type RemoteObjectId = string;

  export type ObjectType = 
    | 'object'
    | 'function'
    | 'undefined'
    | 'string'
    | 'number'
    | 'boolean'
    | 'symbol'
    | 'bigint';

  export type ObjectSubtype = 
    | 'array'
    | 'null'
    | 'node'
    | 'regexp'
    | 'date'
    | 'map'
    | 'set'
    | 'weakmap'
    | 'weakset'
    | 'iterator'
    | 'generator'
    | 'error'
    | 'proxy'
    | 'promise'
    | 'typedarray'
    | 'arraybuffer'
    | 'dataview'
    | 'webassemblymemory'
    | 'wasmvalue';

  export type UnserializableValue = 
    | 'Infinity'
    | 'NaN'
    | '-Infinity'
    | '-0';

  export interface DeepSerializedValue {
    type: ObjectType;
    value?: any;
    objectId?: string;
    weakLocalObjectReference?: number;
  }

  export interface ObjectPreview {
    type: ObjectType;
    subtype?: ObjectSubtype;
    description?: string;
    overflow: boolean;
    properties: PropertyPreview[];
    entries?: EntryPreview[];
  }

  export interface PropertyPreview {
    name: string;
    type: ObjectType;
    value?: string;
    valuePreview?: ObjectPreview;
    subtype?: ObjectSubtype;
  }

  export interface EntryPreview {
    key?: ObjectPreview;
    value: ObjectPreview;
  }

  export interface CustomPreview {
    header: string;
    bodyGetterId?: RemoteObjectId;
  }

  export interface EvaluateResult {
    result: RemoteObject;
    exceptionDetails?: ExceptionDetails;
  }

  export interface ExceptionDetails {
    exceptionId: number;
    text: string;
    lineNumber: number;
    columnNumber: number;
    scriptId?: string;
    url?: string;
    stackTrace?: StackTrace;
    exception?: RemoteObject;
    executionContextId?: number;
    exceptionMetaData?: any;
  }

  export interface StackTrace {
    description?: string;
    callFrames: CallFrame[];
    parent?: StackTrace;
    parentId?: StackTraceId;
  }

  export interface CallFrame {
    functionName: string;
    scriptId: string;
    url: string;
    lineNumber: number;
    columnNumber: number;
  }

  export interface StackTraceId {
    id: string;
    debuggerId?: string;
  }
}

// === Input Domain Types ===

export namespace Input {
  export interface TouchPoint {
    x: number;
    y: number;
    radiusX?: number;
    radiusY?: number;
    rotationAngle?: number;
    force?: number;
    tangentialPressure?: number;
    tiltX?: number;
    tiltY?: number;
    twist?: number;
    id?: number;
  }

  export type GestureSourceType = 'default' | 'touch' | 'mouse';

  export type MouseButton = 'none' | 'left' | 'middle' | 'right' | 'back' | 'forward';

  export interface TimeSinceEpoch {
    timestamp: number;
  }

  export type ModifierBit = 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128 | 256 | 512;
}

// === Accessibility Domain Types ===

export namespace Accessibility {
  export type AXNodeId = string;

  export interface AXNode {
    nodeId: AXNodeId;
    ignored: boolean;
    ignoredReasons?: AXProperty[];
    role?: AXValue;
    chromeRole?: AXValue;
    name?: AXValue;
    description?: AXValue;
    value?: AXValue;
    properties?: AXProperty[];
    parentId?: AXNodeId;
    childIds?: AXNodeId[];
    backendDOMNodeId?: DOM.BackendNodeId;
    frameId?: string;
  }

  export interface AXProperty {
    name: AXPropertyName;
    value: AXValue;
  }

  export interface AXValue {
    type: AXValueType;
    value?: any;
    sources?: AXValueSource[];
  }

  export interface AXValueSource {
    type: AXValueSourceType;
    value?: AXValue;
    attribute?: string;
    attributeValue?: AXValue;
    superseded?: boolean;
    nativeSource?: AXValueNativeSourceType;
    nativeSourceValue?: AXValue;
    invalid?: boolean;
    invalidReason?: string;
  }

  export type AXValueType =
    | 'boolean'
    | 'tristate'
    | 'booleanOrUndefined'
    | 'idref'
    | 'idrefList'
    | 'integer'
    | 'node'
    | 'nodeList'
    | 'number'
    | 'string'
    | 'computedString'
    | 'token'
    | 'tokenList'
    | 'domRelation'
    | 'role'
    | 'internalRole'
    | 'valueUndefined';

  export type AXValueSourceType =
    | 'attribute'
    | 'implicit'
    | 'style'
    | 'contents'
    | 'placeholder'
    | 'relatedElement';

  export type AXValueNativeSourceType =
    | 'description'
    | 'figcaption'
    | 'label'
    | 'labelfor'
    | 'labelwrapped'
    | 'legend'
    | 'rubyannotation'
    | 'tablecaption'
    | 'title'
    | 'other';

  export type AXPropertyName =
    | 'busy'
    | 'disabled'
    | 'editable'
    | 'focusable'
    | 'focused'
    | 'hidden'
    | 'hiddenRoot'
    | 'invalid'
    | 'keyshortcuts'
    | 'settable'
    | 'roledescription'
    | 'live'
    | 'atomic'
    | 'relevant'
    | 'root'
    | 'autocomplete'
    | 'hasPopup'
    | 'level'
    | 'multiselectable'
    | 'orientation'
    | 'multiline'
    | 'readonly'
    | 'required'
    | 'valuemin'
    | 'valuemax'
    | 'valuetext'
    | 'checked'
    | 'expanded'
    | 'modal'
    | 'pressed'
    | 'selected'
    | 'activedescendant'
    | 'controls'
    | 'describedby'
    | 'details'
    | 'errormessage'
    | 'flowto'
    | 'labelledby'
    | 'owns'
    | 'posInSet'
    | 'setSize';
}

// === Event Types ===

export interface CDPEvent {
  method: string;
  params: any;
  sessionId?: string;
}

// === Common Response Types ===

export interface CDPResponse<T = any> {
  id?: number;
  result?: T;
  error?: {
    code: number;
    message: string;
    data?: any;
  };
}

// === Session Types ===

export interface SessionTarget {
  sessionId: string;
  targetInfo: TargetInfo;
}

// === Utility Types ===

export type CDPMethod = string;
export type CDPParams = Record<string, any>;

export interface CDPCommandOptions {
  timeout?: number;
  sessionId?: string;
  frameId?: string;
}

// === Error Types ===

export class CDPError extends Error {
  constructor(
    public code: number,
    message: string,
    public data?: any
  ) {
    super(message);
    this.name = 'CDPError';
  }
}

// === Frame Router Specific Types ===

export interface FrameRoute {
  frameId: string;
  sessionId: string;
  targetId: string;
  parentFrameId?: string;
  url?: string;
  securityOrigin?: string;
}

export interface ExecutionContextInfo {
  contextId: number;
  frameId: string;
  origin: string;
  name: string;
  uniqueId: string;
}

// === Unified Element Types ===

export interface ElementInfo {
  id: string; // Encoded as frameOrdinal:backendNodeId
  frameId: string;
  frameOrdinal: number;
  backendNodeId: DOM.BackendNodeId;
  nodeId?: DOM.NodeId;
  role?: string;
  name?: string;
  url?: string;
  visible?: boolean;
  clickable?: boolean;
  box?: DOM.Rect;
  selector?: string;
  text?: string;
  value?: string;
  attributes?: Record<string, string>;
}

export interface UnifiedSnapshot {
  frames: Array<{
    frameId: string;
    parentId?: string;
    url?: string;
    ordinal: number;
    sessionId?: string;
    securityOrigin?: string;
  }>;
  elements: ElementInfo[];
  createdAt: number;
  viewportSize?: {
    width: number;
    height: number;
  };
}