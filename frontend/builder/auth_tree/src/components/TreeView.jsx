import { useState, useRef, useCallback, useEffect } from 'react';
import { drag } from 'd3-drag';
import { select } from 'd3-selection';

/**
 * TreeView - A drag-and-drop tree component for building Soroban auth hierarchies.
 * 
 * Features:
 * - Drag-and-drop nodes to build authorization trees
 * - Support for arbitrary tree depth
 * - Generate JSON payload for soroban-cli
 * - Validate the constructed tree against Soroban requirements
 */
export default function TreeView() {
  const [nodes, setNodes] = useState([]);
  const [selectedNode, setSelectedNode] = useState(null);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const containerRef = useRef<HTMLDivElement>(null);

  // Initialize with a default node if none exist
  useEffect(() => {
    if (nodes.length === 0) {
      setNodes([{
        id: 'root-1',
        type: 'contract',
        label: 'Root Contract',
        contractId: 'root-contract',
        depth: 0,
        children: [],
      }]);
    }
  }, [nodes.length]);const handleNodeClick = useCallback(function(node) {
      setSelectedNode(node);
    }, []);

  const handleAddNode = useCallback(() => {
    setNodes(prev => {
      const newNode = {
        id: `node-${Date.now()}`,
        type: 'address',
        label: `Address ${prev.length + 1}`,
        depth: (prev.reduce((max, n) => Math.max(max, n.depth || 0), 0) + 1),
      };

      // Add as child of the first selected node, or as root
      if (selectedNode) {
        return prev.map(node =>
          node.id === selectedNode.id
            ? { ...node, children: [...(node.children || []), newNode] }
            : node
        );
      }

      return [...prev, newNode];
    });
  }, [selectedNode]);const handleRemoveNode = useCallback(function(nodeId) {
      setNodes(prev => prev.filter(node => node.id !== nodeId));
      if (selectedNode?.id === nodeId) {
        setSelectedNode(null);
      }
    }, []);

  const handleGeneratePayload = async () => {
    // Transform nodes to the auth tree format for validation
    const authTreeNodes = nodes.map(node => ({
      id: node.id,
      label: node.label,
      type: node.type,
      address: node.type === 'address' ? node.address : undefined,
      contractId: node.type === 'contract' ? node.contractId : undefined,
      subContracts: node.children || [],
      depth: node.depth || 0,
    }));

    // Validate and generate payload
    const { validateAuthTree, generateAuthPayload } = await import('../lib/auth_validator.ts');

    // We'll create a root node from the tree
    const rootNode = {
      id: 'root',
      label: 'Auth Tree Root',
      type: 'contract',
      contractId: 'root-contract',
      depth: 0,
      subContracts: authTreeNodes,
    };

    const result = generateAuthPayload(rootNode);

    if (result) {
      alert('Auth Payload Generated:\n' + JSON.stringify(result, null, 2));
    } else {
      alert('Failed to generate valid auth payload. Check the tree structure.');
    }
  };

  const handleValidateTree = async () => {
    const authTreeNodes = nodes.map(node => ({
      id: node.id,
      label: node.label,
      type: node.type,
      address: node.type === 'address' ? node.address : undefined,
      contractId: node.type === 'contract' ? node.contractId : undefined,
      subContracts: node.children || [],
      depth: node.depth || 0,
    }));

    const rootNode = {
      id: 'root',
      label: 'Auth Tree Root',
      type: 'contract',
      contractId: 'root-contract',
      depth: 0,
      subContracts: authTreeNodes,
    };

    const { validateAuthTree } = await import('../lib/auth_validator.ts');
    const result = validateAuthTree(rootNode);

    const msg = result.valid
      ? 'Auth tree is valid!'
      : `Auth tree has ${result.errors.length} error(s):\n${result.errors.join('\n')}`;

    alert(msg);

    if (result.payload) {
      alert('Generated Payload:\n' + JSON.stringify(result.payload, null, 2));
    }
  };

  // Add default scenario: Alice -> Router -> Liquidity Pool
  useEffect(() => {
    // Check if we should add the default scenario
    const hasDefaultScenario = nodes.some(
      node => node.label && node.label.includes('Alice') && node.label.includes('Router')
    );

    if (!hasDefaultScenario && nodes.length <= 1) {
      setNodes(prev => {
        const aliceAddress = "GABCDEFGHIJKLMNOPQRSTUVWXYZ1234567";

        return [
          {
            id: 'router-1',
            type: 'contract',
            label: 'Router Contract',
            contractId: 'router-id',
            depth: 0,
            children: [
              {
                id: 'pool-1',
                type: 'contract',
                label: 'Liquidity Pool',
                contractId: 'pool-id',
                depth: 1,
                children: [
                  {
                    id: 'alice-1',
                    type: 'address',
                    label: 'Alice',
                    address: aliceAddress,
                    depth: 2,
                  },
                ],
              },
            ],
          },
        ];
      });
    }
  }, [nodes.length]);

  return (
    <div className="tree-view-container" 
         ref={containerRef}
         style={{ 
           width: '100%', 
           height: '800px',
           border: '1px solid #ddd',
           overflow: 'auto',
           fontFamily: 'sans-serif',
         }}>
      <header style={{ padding: '8px 12px', background: '#f8f9fa', borderBottom: '1px solid #dee2e3' }}>
        <h2 style={{ margin: 0, display: 'inline-block' }}>Soroban Auth Hierarchy Builder</h2>
        <div style={{ display: 'inline-block', float: 'right', marginLeft: '16px' }}><button
            onClick={() => handleAddNode()}
            style={{ marginRight: '8px' }}
            title="Add Node"
          >
            + Add
          </button>
          <button 
            onClick={handleGeneratePayload}
            title="Generate Payload"
          >
            Generate
          </button>
          <button 
            onClick={handleValidateTree}
            title="Validate Tree"
          >
            Validate
          </button>
        </div>
      </header>

      <div 
        style={{
          position: 'relative',
          width: '100%',
          height: 'calc(800px - 40px)',
          background: '#fff',
        }}
      >
        {/* Render tree nodes */}
        {nodes.length > 0 && renderNodes(nodes, containerRef.current)}
      </div>
    </div>
  );
}

/**
 * Render the tree nodes with drag-and-drop support
 */
function renderNodes(nodes, container) {
  const svg = select(container).select('svg') || select(container).append('svg');

  // Set SVG dimensions
  const containerRect = container.getBoundingClientRect();
  svg.attr('width', containerRect.width).attr('height', containerRect.height);

  // Apply zoom and pan transforms
  const g = svg.append('g')
    .attr('transform', `translate(${pan.x}, ${pan.y}) scale(${zoom})`);

  // Node dimensions
  const nodeWidth = 120;
  const nodeHeight = 40;
  const nodeRadius = 8;
  const childrenPadding = 40;
  const horizontalPadding = 60;

  // Calculate tree layout using D3-like hierarchy
  const treeLayout = buildTreeLayout(nodes, nodeWidth, nodeHeight);

  // Draw connections between nodes
  const links = treeLayout.links;
  const nodesRender = treeLayout.nodes;

  // Draw links (connections)
  const linkEnter = g.selectAll('.link')
    .data(links)
    .enter()
    .append('path')
    .attr('class', 'link')
    .attr('fill', 'none')
    .attr('stroke', '#ccc')
    .attr('stroke-width', 2)
    .attr('d', d => `M${d.source.x},${d.source.y}L${d.target.x},${d.target.y}`);

  // Draw nodes
  const nodeEnter = g.selectAll('.node')
    .data(nodesRender)
    .enter()
    .append('g')
    .attr('class', 'node')
    .attr('cursor', 'pointer')
    .attr('transform', d => `translate(${d.x}, ${d.y})`)
    .on('click', (event, d) => {
      // Handle node click
    });

  // Draw node rectangles
  nodeEnter.append('rect')
    .attr('width', nodeWidth)
    .attr('height', nodeHeight)
    .attr('rx', nodeRadius)
    .attr('ry', nodeRadius)
    .attr('fill', d => d.type === 'contract' ? '#3b82f6' : '#10b981')
    .attr('stroke', '#fff')
    .attr('stroke-width', 2);

  // Draw node labels
  nodeEnter.append('text')
    .attr('x', nodeWidth / 2)
    .attr('y', nodeHeight / 2 + 5)
    .attr('text-anchor', 'middle')
    .attr('fill', '#fff')
    .attr('font-size', '12px')
    .attr('font-weight', 'bold')
    .text(d => d.label.length > 20 ? d.label.substring(0, 17) + '...' : d.label);

  // Add drag behavior to nodes
  nodeEnter.call(drag()
    .subject(d => ({ x: d.x, y: d.y }))
    .on('start', (event, d) => {
      // Drag start logic
    })
    .on('drag', (event, d) => {
      d.x = event.x;
      d.y = event.y;
      nodeEnter.attr('transform', d => `translate(${d.x}, ${d.y})`);
    }));

  // Return the g element for potential further manipulation
  return g;
}

/**
 * Builds a tree layout from the nodes array.
 * Calculates positions for each node based on tree structure.
 */
function buildTreeLayout(nodes, nodeWidth, nodeHeight) {
  const nodesMap = new Map();
  const links = [];

  // First pass: create map of all nodes
  function addNodeToMap(node, parentId) {
    if (!node.id) {
      node.id = `tmp-${Math.random().toString(36).substr(2, 9)}`;
    }

    nodesMap.set(node.id, {
      ...node,
      x: 0,
      y: 0,
      children: node.children || [],
    });

    if (parentId && nodesMap.has(parentId)) {
      const parent = nodesMap.get(parentId);
      if (!parent.children.includes(node.id)) {
        parent.children.push(node.id);
      }
    }

    // Create link from parent to this node
    if (parentId) {
      links.push({ source: nodesMap.get(parentId), target: nodesMap.get(node.id) });
    }

    // Recursively add children
    node.children?.forEach(function(childId) {
      addNodeToMap(nodesMap.get(childId), node.id);
    });

    return nodesMap.get(node.id);
  }

  // Start with root nodes
  const rootNodes = [];
  nodes.forEach(node => {
    const added = addNodeToMap(nodesMap.get(node.id), undefined);
    if (!node.parentId) {
      rootNodes.push(added);
    }
  });

  // Second pass: calculate positions using BFS/DFS
  let xCounter = 0;
  let yCounter = 0;

  function assignPositions(nodes, x, y, depth) {
    depth = depth || 0;
    nodes.forEach(function(node) {
      node.x = x;
      node.y = y + depth * (nodeHeight + childrenPadding);
    });

    // Collect all children
    const allChildren = [];
    nodes.forEach(function(node) {
      allChildren.push(...node.children.map(function(id) { return nodesMap.get(id); }).filter(function(child) { return child; }));
    });

    if (allChildren.length > 0) {
      assignPositions(allChildren, x + nodeWidth + horizontalPadding, y, depth + 1);
    }
  }

  assignPositions(rootNodes, 0, 0, 0);

  return { nodes: rootNodes.flatMap(n => getAllNodes(n, nodesMap)), links };
}

/**
 * Helper to get all nodes recursively
 */
function getAllNodes(node, nodesMap) {
  const result = [node];
  node.children?.forEach(function(id) {
    result.push(...getAllNodes(nodesMap.get(id), nodesMap));
  });
  return result;
}

/**
 * Export the current tree as JSON payload for soroban-cli
 */
function exportPayload(nodes) {
  const authTreeNodes = nodes.map(function(node) {
    return {
      id: node.id,
      label: node.label,
      type: node.type,
      address: node.type === 'address' ? node.address : undefined,
      contractId: node.type === 'contract' ? node.contractId : undefined,
      subContracts: node.children || [],
      depth: node.depth || 0,
    };
  });

  const rootNode = {
    id: 'root',
    label: 'Auth Tree Root',
    type: 'contract',
    contractId: 'root-contract',
    depth: 0,
    subContracts: authTreeNodes,
  };

  const { generateAuthPayload } = require('../lib/auth_validator.ts');
  return generateAuthPayload(rootNode);
}