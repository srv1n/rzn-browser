import { readFileSync, writeFileSync, mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { createHash } from 'crypto';

const __dirname = dirname(fileURLToPath(import.meta.url));

// Read the JSON schema
const schemaPath = join(__dirname, '../../schema/actions-v1.json');
const schemaContent = readFileSync(schemaPath, 'utf8');
const schema = JSON.parse(schemaContent);
const schemaHash = createHash('sha256').update(schemaContent).digest('hex');
const schemaVersion = schema.schema_version ?? 'unknown';

// Generate TypeScript types from JSON schema
function generateTypeScript(schema: any): string {
  let code = `// Auto-generated from schema/actions-v1.json
// schema-version: ${schemaVersion}
// schema-sha256: ${schemaHash}
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

`;

  // Generate action types
  const actionTypes: string[] = [];
  
  if (schema.oneOf) {
    for (const actionRef of schema.oneOf) {
      const refPath = actionRef.$ref.split('/');
      const actionName = refPath[refPath.length - 1];
      const definition = schema.definitions[actionName];
      const actionDef = definition?.[actionName] ?? definition;

      if (actionDef) {
        const typeName = toPascalCase(actionName);
        const properties = actionDef?.properties || {};
        const required = actionDef?.required || [];
        const allowAdditional = actionDef?.additionalProperties === true;
        
        code += `export interface ${typeName} {\n`;
        code += `  type: '${actionName}';\n`;
        
        for (const [propName, propDef] of Object.entries(properties as any)) {
          if (propName === 'type') continue;
          const isRequired = required.includes(propName);
          const propType = getTypeScriptType(propDef);
          code += `  ${propName}${isRequired ? '' : '?'}: ${propType};\n`;
        }
        if (allowAdditional) {
          code += `  [key: string]: any;\n`;
        }
        
        code += `}\n\n`;
        actionTypes.push(typeName);
      }
    }
  }
  
  // Generate union type
  code += `export type Action = \n  | ${actionTypes.join('\n  | ')};\n\n`;
  
  // Generate type guards
  code += `// Type guards\n`;
  for (const actionType of actionTypes) {
    const actionName = toSnakeCase(actionType);
    code += `export function is${actionType}(action: Action): action is ${actionType} {
  return action.type === '${actionName}';
}\n\n`;
  }
  
  // Generate Zod schemas
  code += generateZodSchemas(schema);
  
  return code;
}

function generateZodSchemas(schema: any): string {
  let code = `\n// Zod schemas for runtime validation\nimport { z } from 'zod';\n\n`;
  
  // RobustSelectors schema
  code += `export const RobustSelectorsSchema = z.object({
  primary: z.string().optional(),
  fallbacks: z.array(z.string()).optional(),
  confidence: z.number().min(0).max(1).optional(),
  visualHash: z.string().regex(/^[A-Fa-f0-9]{64}$/).optional(),
});\n\n`;
  
  // Selectors schema
  code += `export const SelectorsSchema = z.object({
  css: z.string().optional(),
  xpath: z.string().optional(),
  text: z.string().optional(),
  robust: RobustSelectorsSchema.optional(),
});\n\n`;

  // FieldSpec schema
  code += `export const FieldSpecSchema = z.object({
  name: z.string(),
  selector: z.string(),
  attribute: z.string().optional(),
  post_processing: z.array(z.string()).optional(),
}).passthrough();\n\n`;

  // CookieSpec schema
  code += `export const CookieSpecSchema = z.object({
  name: z.string(),
  value: z.string(),
  domain: z.string().optional(),
  path: z.string().optional(),
  secure: z.boolean().optional(),
  http_only: z.boolean().optional(),
  expiration_date: z.number().optional(),
}).passthrough();\n\n`;
  
  // Generate individual action schemas
  const actionSchemas: string[] = [];
  
  if (schema.oneOf) {
    for (const actionRef of schema.oneOf) {
      const refPath = actionRef.$ref.split('/');
      const actionName = refPath[refPath.length - 1];
      const definition = schema.definitions[actionName];
      const actionDef = definition?.[actionName] ?? definition;
      
      if (actionDef) {
        const schemaName = `${toPascalCase(actionName)}Schema`;
        const properties = actionDef?.properties || {};
        const required = actionDef?.required || [];
        const allowAdditional = actionDef?.additionalProperties === true;
        
        code += `export const ${schemaName} = z.object({\n`;
        code += `  type: z.literal('${actionName}'),\n`;
        
        for (const [propName, propDef] of Object.entries(properties as any)) {
          if (propName === 'type') continue;
          const isRequired = required.includes(propName);
          const zodType = getZodType(propDef);
          code += `  ${propName}: ${zodType}${isRequired ? '' : '.optional()'},\n`;
        }
        
        code += `})${allowAdditional ? '.passthrough()' : ''};\n\n`;
        actionSchemas.push(schemaName);
      }
    }
  }
  
  // Generate union schema
  code += `export const ActionSchema = z.discriminatedUnion('type', [\n`;
  code += actionSchemas.map(s => `  ${s},`).join('\n');
  code += `\n]);\n\n`;
  
  code += `export type ActionFromSchema = z.infer<typeof ActionSchema>;\n`;
  
  return code;
}

function getTypeScriptType(propDef: any): string {
  if (propDef.type === 'string') {
    if (propDef.enum) {
      return propDef.enum.map((v: string) => `'${v}'`).join(' | ');
    }
    return 'string';
  }
  if (propDef.type === 'integer' || propDef.type === 'number') {
    return 'number';
  }
  if (propDef.type === 'boolean') {
    return 'boolean';
  }
  if (propDef.type === 'array') {
    const itemType = propDef.items ? getTypeScriptType(propDef.items) : 'any';
    return `${itemType}[]`;
  }
  if (propDef.type === 'object') {
    if (propDef.$ref) {
      const refName = propDef.$ref.split('/').pop();
      return toPascalCase(refName);
    }
    if (propDef.additionalProperties) {
      const valueType = getTypeScriptType(propDef.additionalProperties);
      return `Record<string, ${valueType}>`;
    }
    if (propDef.properties) {
      // Inline object type
      const props = Object.entries(propDef.properties)
        .map(([k, v]) => `${k}: ${getTypeScriptType(v)}`)
        .join('; ');
      return `{ ${props} }`;
    }
    return 'any';
  }
  return 'any';
}

function getZodType(propDef: any): string {
  if (propDef.type === 'string') {
    let zodType = 'z.string()';
    if (propDef.enum) {
      const enumValues = propDef.enum.map((v: string) => `'${v}'`).join(', ');
      zodType = `z.enum([${enumValues}])`;
    }
    if (propDef.format === 'uri') {
      zodType += '.url()';
    }
    if (propDef.pattern) {
      zodType += `.regex(/${propDef.pattern}/)`;
    }
    return zodType;
  }
  if (propDef.type === 'integer') {
    let zodType = 'z.number().int()';
    if (propDef.minimum !== undefined) {
      zodType += `.min(${propDef.minimum})`;
    }
    if (propDef.maximum !== undefined) {
      zodType += `.max(${propDef.maximum})`;
    }
    return zodType;
  }
  if (propDef.type === 'number') {
    let zodType = 'z.number()';
    if (propDef.minimum !== undefined) {
      zodType += `.min(${propDef.minimum})`;
    }
    if (propDef.maximum !== undefined) {
      zodType += `.max(${propDef.maximum})`;
    }
    return zodType;
  }
  if (propDef.type === 'boolean') {
    return 'z.boolean()';
  }
  if (propDef.type === 'array') {
    const itemType = propDef.items ? getZodType(propDef.items) : 'z.any()';
    let zodType = `z.array(${itemType})`;
    if (propDef.maxItems !== undefined) {
      zodType += `.max(${propDef.maxItems})`;
    }
    return zodType;
  }
  if (propDef.type === 'object') {
    if (propDef.$ref === '#/definitions/robustSelectors') {
      return 'RobustSelectorsSchema';
    }
    if (propDef.$ref === '#/definitions/field_spec') {
      return 'FieldSpecSchema';
    }
    if (propDef.$ref === '#/definitions/cookie_spec') {
      return 'CookieSpecSchema';
    }
    if (propDef.additionalProperties) {
      const valueType = getZodType(propDef.additionalProperties);
      return `z.record(${valueType})`;
    }
    if (propDef.properties) {
      // Inline object
      const props = Object.entries(propDef.properties)
        .map(([k, v]) => `${k}: ${getZodType(v)}`)
        .join(', ');
      return `z.object({ ${props} })`;
    }
    return 'z.any()';
  }
  return 'z.any()';
}

function toPascalCase(str: string): string {
  return str
    .split('_')
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join('');
}

function toSnakeCase(str: string): string {
  return str
    .replace(/([A-Z])/g, '_$1')
    .toLowerCase()
    .replace(/^_/, '');
}

// Generate the TypeScript code
const tsCode = generateTypeScript(schema);

// Ensure the output directory exists
const outputDir = join(__dirname, '../src/types');
mkdirSync(outputDir, { recursive: true });

// Write the generated file
const outputPath = join(outputDir, 'actions.ts');
writeFileSync(outputPath, tsCode);

console.log(`Generated TypeScript types at: ${outputPath}`);
