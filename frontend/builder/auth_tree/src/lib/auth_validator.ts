/**
 * Auth Validator for Soroban Authorization Hierarchies
 * 
 * Validates that constructed auth trees satisfy Soroban's require_auth framework
 * requirements for nested authorization trees.
 */

import { sortBy } from 'lodash';

/**
 * AuthTreeNode represents a node in the authorization tree.
 * A node can be an address (user or contract) or a contract with sub-contracts.
 */
export interface AuthTreeNode {
  id: string;
  label: string;
  type: 'address' | 'contract';
  address?: string; // Soroban address (public key hash)
  contractId?: string; // Contract ID if type is 'contract'
  subContracts?: AuthTreeNode[];
  depth: number;
}

/**
 * AuthPayload represents the JSON payload generated for soroban-cli.
 * This matches the format expected by soroban auth payload generation.
 */
export interface AuthPayload {
  source: string; // The root contract or address initiating the auth
  actions: AuthAction[];
  resource: string; // Resource being accessed
}

/**
 * AuthAction represents a single authorization action in the payload.
 */
export interface AuthAction {
  address: string; // Address being authorized
  permission: 'invoke' | 'edit' | 'admin';
}

/**
 * Validates that the constructed auth tree satisfies Soroban's requirements.
 * Returns validation results with any errors found.
 */
export function validateAuthTree(root: AuthTreeNode): {
  valid: boolean;
  errors: string[];
  payload: AuthPayload | null;
} {
  const errors: string[] = [];
  const actions: AuthAction[] = [];
  
  // Validate the tree structure and collect actions
  const validateNode = (node: AuthTreeNode, visited = new Set<string>()): boolean => {
    if (!node || visited.has(node.id)) {
      return true; // Skip null or already visited nodes
    }
    
    visited.add(node.id);
    
    // Validate node has required fields
    if (!node.label) {
      errors.push(`Node ${node.id} is missing a label`);
      return false;
    }
    
    // If it's a contract node, validate sub-contracts
    if (node.type === 'contract' && node.subContracts) {
      for (const subNode of node.subContracts) {
        if (!validateNode(subNode, visited)) {
          errors.push(`Invalid sub-contract in node ${node.id}`);
        }
      }
    }
    
    // Collect auth actions for address nodes
    if (node.type === 'address' && node.address) {
      actions.push({
        address: node.address,
        permission: 'invoke', // Default permission
      });
    }
    
    return true;
  };
  
  validateNode(root);
  
  // Build the auth payload
  let payload: AuthPayload | null = null;
  
  // Find the root (the node with no parent or the top-level contract)
  const findRoot = (node: AuthTreeNode): AuthTreeNode => {
    if (node.type === 'contract' && node.subContracts) {
      // The root contract is the one that has sub-contracts calling it
      // or it's the topmost node in the tree
      let foundRoot = node;
      if (node.subContracts) {
        for (const sub of node.subContracts) {
          const subRoot = findRoot(sub);
          // Use the topmost root (first one found)
          if (!foundRoot.contractId) {
            foundRoot = subRoot;
          }
        }
      }
      return foundRoot;
    }
    return node;
  };
  
  const rootNode = findRoot(root);
  
  // Build payload from the validated tree
  if (rootNode.type === 'contract') {
    payload = {
      source: rootNode.address || rootNode.label,
      actions,
      resource: rootNode.contractId || 'unknown-contract',
    };
  } else if (rootNode.type === 'address') {
    payload = {
      source: rootNode.address || rootNode.label,
      actions,
      resource: 'unknown-contract',
    };
  }
  
  // Check for minimum requirements
  if (actions.length === 0) {
    errors.push('No authorization actions defined in the tree');
    payload = null;
  }
  
  if (!rootNode.label) {
    errors.push('Auth tree must have at least one root node with a label');
    payload = null;
  }
  
  // Sort actions by address for consistent output
  if (payload?.actions) {
    payload.actions = sortBy(payload.actions, 'address');
  }
  
  return {
    valid: errors.length === 0,
    errors,
    payload,
  };
}

/**
 * Generates the JSON payload required for soroban-cli to sign and execute
 * a multi-step transaction based on the constructed auth tree.
 * 
 * The payload format follows Soroban's authorization model where:
 * - source is the root contract/address
 * - actions are the required authorizations
 * - resource specifies what is being accessed
 */
export function generateAuthPayload(root: AuthTreeNode): AuthPayload | null {
  const { valid, errors, payload } = validateAuthTree(root);
  
  if (!valid) {
    console.error('Auth tree validation failed:', errors);
    return null;
  }
  
  if (!payload) {
    console.error('Failed to generate auth payload');
    return null;
  }
  
  return payload;
}

/**
 * Creates a sample auth tree for the Alice-Router-Liquidity Pool scenario.
 * 
 * Scenario: Alice calls a Router which calls a Liquidity Pool
 * 
 * This demonstrates the validation and payload generation for a real-world
 * Soroban DeFi authorization hierarchy.
 */
export function createAliceRouterLiquidityPoolTree(): AuthTreeNode {
  // Alice's address (example Soroban public key hash)
  const aliceAddress = "GABCDEFGHIJKLMNOPQRSTUVWXYZ1234567";
  
  // Router contract
  const routerContract = {
    id: "router-1",
    label: "Router Contract",
    type: "contract",
    contractId: "router-id",
    depth: 0,
    subContracts: [
      // Liquidity Pool contract
      {
        id: "pool-1",
        label: "Liquidity Pool",
        type: "contract",
        contractId: "pool-id",
        depth: 1,
        subContracts: [
          // Alice's address as an authorized participant
          {
            id: "alice-1",
            label: "Alice",
            type: "address",
            address: aliceAddress,
            depth: 2,
          },
        ],
      },
    ],
  };
  
  return routerContract;
}

/**
 * Creates a more complex auth tree with arbitrary depth.
 * 
 * @param depth The maximum depth of the tree
 * @returns A nested auth tree structure
 */
export function createDeepAuthTree(depth: number): AuthTreeNode {
  if (depth <= 0) {
    return {
      id: `leaf-${Math.random().toString(36).substr(2, 9)}`,
      label: `Level ${depth}`,
      type: "address",
      address: `addr-${Math.random().toString(36).substr(2, 12)}`,
      depth: 0,
    };
  }
  
  return {
    id: `node-${Math.random().toString(36).substr(2, 9)}`,
    label: `Contract Level ${depth}`,
    type: "contract",
    contractId: `contract-${depth}`,
    depth: 0,
    subContracts: [createDeepAuthTree(depth - 1)],
  };
}

export default { validateAuthTree, generateAuthTreePayload: generateAuthPayload, createAliceRouterLiquidityPoolTree, createDeepAuthTree };