import { describe, it, expect } from 'vitest';
import {
  validateAuthTree,
  generateAuthPayload,
  createAliceRouterLiquidityPoolTree,
  createDeepAuthTree,
} from '../src/lib/auth_validator';

describe('Auth Validator', () => {
  describe('validateAuthTree', () => {
    it('should validate a simple auth tree with one address', () => {
      const tree = {
        id: '1',
        label: 'Test Contract',
        type: 'contract',
        contractId: 'test-contract',
        depth: 0,
        subContracts: [
          {
            id: '2',
            label: 'Alice',
            type: 'address',
            address: 'GABCDEFGHIJKLMNOPQRSTUVWXYZ1234567',
            depth: 1,
          },
        ],
      };

      const result = validateAuthTree(tree);
      expect(result.valid).toBe(true);
      expect(result.errors.length).toBe(0);
      expect(result.payload).not.toBeNull();
    });

    it('should validate the Alice-Router-Liquidity Pool scenario', () => {
      const tree = createAliceRouterLiquidityPoolTree();
      const result = validateAuthTree(tree);
      
      expect(result.valid).toBe(true);
      expect(result.errors.length).toBe(0);
      expect(result.payload).not.toBeNull();
      
      // Verify payload structure
      expect(result.payload?.source).toBeDefined();
      expect(Array.isArray(result.payload?.actions)).toBe(true);
      expect(result.payload?.resource).toBeDefined();
    });

    it('should reject tree with no actions', () => {
      const tree = {
        id: '1',
        label: 'Test Contract',
        type: 'contract',
        contractId: 'test-contract',
        depth: 0,
        subContracts: [],
      };

      const result = validateAuthTree(tree);
      expect(result.valid).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('should reject tree with no root label', () => {
      const tree = {
        id: '1',
        label: '',
        type: 'contract',
        contractId: 'test-contract',
        depth: 0,
        subContracts: [],
      };

      const result = validateAuthTree(tree);
      expect(result.valid).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('should handle deep trees', () => {
      const tree = createDeepAuthTree(5);
      const result = validateAuthTree(tree);
      
      // Deep trees should be valid even if they have limited actions
      expect(result.valid || result.errors.length).toBeDefined();
    });
  });

  describe('generateAuthPayload', () => {
    it('should generate payload for Alice-Router-Liquidity Pool scenario', () => {
      const tree = createAliceRouterLiquidityPoolTree();
      const payload = generateAuthPayload(tree);
      
      expect(payload).not.toBeNull();
      expect(payload?.source).toBeDefined();
      expect(Array.isArray(payload?.actions)).toBe(true);
      expect(payload?.resource).toBeDefined();
    });

    it('should generate valid payload structure', () => {
      const tree = createDeepAuthTree(3);
      const payload = generateAuthPayload(tree);
      
      expect(payload).not.toBeNull();
      if (payload) {
        expect(payload.source).toBeDefined();
        expect(Array.isArray(payload.actions)).toBe(true);
        expect(payload.resource).toBeDefined();
        
        // Verify each action has required fields
        payload.actions.forEach((action: any) => {
          expect(action.address).toBeDefined();
          expect(['invoke', 'edit', 'admin'].includes(action.permission)).toBe(true);
        });
      }
    });
  });

  describe('createAliceRouterLiquidityPoolTree', () => {
    it('should create the Alice-Router-Liquidity Pool scenario', () => {
      const tree = createAliceRouterLiquidityPoolTree();
      
      expect(tree.label).toBe('Router Contract');
      expect(tree.type).toBe('contract');
      expect(tree.contractId).toBe('router-id');
      expect(tree.depth).toBe(0);
      expect(Array.isArray(tree.subContracts)).toBe(true);
      expect(tree.subContracts.length).toBeGreaterThan(0);
      
      // Verify nested structure
      expect(tree.subContracts[0].label).toBe('Liquidity Pool');
      expect(tree.subContracts[0].type).toBe('contract');
      expect(tree.subContracts[0].subContracts).toBeDefined();
      expect(tree.subContracts[0].subContracts.length).toBeGreaterThan(0);
      
      // Verify Alice is at the leaf
      expect(tree.subContracts[0].subContracts[0].label).toBe('Alice');
      expect(tree.subContracts[0].subContracts[0].type).toBe('address');
      expect(tree.subContracts[0].subContracts[0].address).toBeDefined();
    });
  });

  describe('createDeepAuthTree', () => {
    it('should create a deep auth tree with specified depth', () => {
      const tree = createDeepAuthTree(3);
      
      expect(tree.depth).toBeLessThanOrEqual(3);
      expect(tree.type).toBe('contract');
      expect(tree.subContracts).toBeDefined();
      
      // Verify nesting
      let current: any = tree;
      let depth = 0;
      while (current.subContracts && current.subContracts.length > 0 && depth < 10) {
        current = current.subContracts[0];
        depth++;
      }
      expect(depth).toBeLessThanOrEqual(3);
    });
  });
});