const FLUENT_HOOKS = new Set(['use', 'useOne', 'useMutation']);
const CONDITIONAL_ANCESTORS = new Set([
  'ArrowFunctionExpression',
  'ConditionalExpression',
  'DoWhileStatement',
  'ForInStatement',
  'ForOfStatement',
  'ForStatement',
  'FunctionDeclaration',
  'FunctionExpression',
  'IfStatement',
  'LogicalExpression',
  'SwitchCase',
  'TryStatement',
  'WhileStatement',
]);

function memberNames(node) {
  if (!node) return [];
  if (node.type === 'ChainExpression') return memberNames(node.expression);
  if (node.type === 'Identifier') return [node.name];
  if (node.type !== 'MemberExpression') return [];
  const property = !node.computed && node.property.type === 'Identifier'
    ? node.property.name
    : node.computed && node.property.type === 'Literal' && typeof node.property.value === 'string'
      ? node.property.value
      : null;
  if (property === null) return [];
  return [...memberNames(node.object), property];
}

function rootIdentifier(node) {
  if (!node) return null;
  if (node.type === 'ChainExpression') return rootIdentifier(node.expression);
  if (node.type === 'Identifier') return node;
  if (node.type === 'MemberExpression') return rootIdentifier(node.object);
  return null;
}

function bindingCallsUseArete(context, identifier) {
  const sourceCode = context.sourceCode ?? context.getSourceCode();
  let scope = sourceCode.getScope(identifier);
  while (scope) {
    const variable = scope.set.get(identifier.name);
    const definition = variable?.defs?.[0];
    const initializer = definition?.node?.type === 'VariableDeclarator'
      ? definition.node.init
      : null;
    if (initializer?.type === 'CallExpression') {
      const calleeNames = memberNames(initializer.callee);
      if (calleeNames[calleeNames.length - 1] === 'useArete') return true;
    }
    scope = scope.upper;
  }
  return false;
}

function isAreteHook(context, node) {
  const names = memberNames(node);
  const hook = names[names.length - 1];
  if (!FLUENT_HOOKS.has(hook)) return false;
  const root = rootIdentifier(node);
  if (!root || (root.name !== 'arete' && !bindingCallsUseArete(context, root))) {
    return false;
  }
  if (hook === 'useMutation') return names.includes('programs');
  return names.includes('views') || names.includes('read') || names.includes('reads');
}

function functionName(node) {
  if (node.type === 'FunctionDeclaration' && node.id) return node.id.name;
  const parent = node.parent;
  if (parent?.type === 'VariableDeclarator' && parent.id.type === 'Identifier') {
    return parent.id.name;
  }
  if (parent?.type === 'Property' && !parent.computed && parent.key.type === 'Identifier') {
    return parent.key.name;
  }
  return null;
}

function isComponentOrHook(node) {
  const name = functionName(node);
  return Boolean(name && (/^[A-Z]/.test(name) || /^use[A-Z0-9]/.test(name)));
}

function containsReturn(node) {
  if (!node || typeof node !== 'object') return false;
  if (node.type === 'ReturnStatement') return true;
  if (
    node.type === 'ArrowFunctionExpression'
    || node.type === 'FunctionDeclaration'
    || node.type === 'FunctionExpression'
  ) {
    return false;
  }
  for (const [key, value] of Object.entries(node)) {
    if (key === 'parent' || key === 'loc' || key === 'range') continue;
    if (Array.isArray(value) && value.some(containsReturn)) return true;
    if (value && typeof value === 'object' && containsReturn(value)) return true;
  }
  return false;
}

module.exports = {
  meta: {
    type: 'problem',
    docs: {
      description: 'enforce React hook ordering for Arete fluent hooks',
    },
    schema: [],
    messages: {
      outsideComponent: 'Arete hook "{{hook}}" must be called from a React component or custom hook.',
      conditional: 'Arete hook "{{hook}}" must be called unconditionally at the top level.',
      afterReturn: 'Arete hook "{{hook}}" cannot be called after an earlier conditional return.',
    },
  },
  create(context) {
    return {
      CallExpression(node) {
        if (!isAreteHook(context, node.callee)) return;

        const hook = context.getSourceCode().getText(node.callee);
        let current = node.parent;
        let owner = null;
        let conditional = false;

        while (current) {
          if (
            current.type === 'ArrowFunctionExpression'
            || current.type === 'FunctionDeclaration'
            || current.type === 'FunctionExpression'
          ) {
            owner = current;
            break;
          }
          if (CONDITIONAL_ANCESTORS.has(current.type)) conditional = true;
          current = current.parent;
        }

        if (!owner || !isComponentOrHook(owner)) {
          context.report({ node, messageId: 'outsideComponent', data: { hook } });
          return;
        }

        if (conditional || owner.body.type !== 'BlockStatement') {
          context.report({ node, messageId: 'conditional', data: { hook } });
          return;
        }

        let statement = node;
        while (statement.parent && statement.parent !== owner.body) {
          if (CONDITIONAL_ANCESTORS.has(statement.parent.type)) {
            context.report({ node, messageId: 'conditional', data: { hook } });
            return;
          }
          statement = statement.parent;
        }

        const index = owner.body.body.indexOf(statement);
        if (index < 0) {
          context.report({ node, messageId: 'conditional', data: { hook } });
          return;
        }
        if (owner.body.body.slice(0, index).some(containsReturn)) {
          context.report({ node, messageId: 'afterReturn', data: { hook } });
        }
      },
    };
  },
};
