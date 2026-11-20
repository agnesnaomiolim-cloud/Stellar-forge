import { useCallback, useMemo, useRef, useState } from 'react';
import {
  createAliceRouterLiquidityPoolTree,
  generateAuthPayload,
  validateAuthTree,
} from '../lib/auth_validator';

const NODE_WIDTH = 150;
const NODE_HEIGHT = 48;
const HORIZONTAL_GAP = 220;
const VERTICAL_GAP = 80;

export default function TreeView() {
  const [nodes, setNodes] = useState([createAliceRouterLiquidityPoolTree()]);
  const [selectedId, setSelectedId] = useState('pool-1');
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 60, y: 50 });
  const [draggingNode, setDraggingNode] = useState(null);
  const [validation, setValidation] = useState(null);
  const svgRef = useRef(null);

  const root = nodes.length === 1 ? nodes[0] : {
    id: 'root-synthetic',
    label: 'Auth Tree Root',
    type: 'contract',
    contractId: 'root-contract',
    depth: 0,
    subContracts: nodes,
  };

  const positions = useMemo(() => layoutTree(nodes), [nodes]);
  const selectedNode = useMemo(() => findNode(nodes, selectedId), [nodes, selectedId]);

  const updateNode = useCallback((id, patch) => {
    setNodes((current) => mapNode(current, (node) =>
      node.id === id ? { ...node, ...patch } : node,
    ));
  }, []);

  const removeNode = useCallback((id) => {
    setNodes((current) => filterNode(current, id));
    setSelectedId(null);
  }, []);

  const addContract = useCallback(() => {
    const parent = selectedNode ?? nodes[0];
    if (!parent) return;
    if (parent.type === 'address') return;
    const id = `contract-${Date.now()}`;
    const child = {
      id,
      label: `Sub-contract ${parent.subContracts?.length ?? 0 + 1}`,
      type: 'contract',
      contractId: `C${'X'.repeat(55)}`,
      depth: (parent.depth ?? 0) + 1,
      subContracts: [],
    };
    setNodes((current) => mapNode(current, (node) =>
      node.id === parent.id
        ? { ...node, subContracts: [...(node.subContracts ?? []), child] }
        : node,
    ));
    setSelectedId(id);
  }, [nodes, selectedNode]);

  const addAddress = useCallback(() => {
    const parent = selectedNode ?? nodes[0];
    if (!parent || parent.type === 'address') return;
    const id = `address-${Date.now()}`;
    const child = {
      id,
      label: `Alice ${parent.subContracts?.length ?? 0 + 1}`,
      type: 'address',
      address: `G${'A'.repeat(55)}`,
      depth: (parent.depth ?? 0) + 1,
    };
    setNodes((current) => mapNode(current, (node) =>
      node.id === parent.id
        ? { ...node, subContracts: [...(node.subContracts ?? []), child] }
        : node,
    ));
    setSelectedId(id);
  }, [nodes, selectedNode]);

  const moveNode = useCallback((nodeId, targetId) => {
    if (nodeId === targetId) return;
    setNodes((current) => {
      const moving = findNode(current, nodeId);
      if (!moving) return current;
      const target = findNode(current, targetId);
      if (!target || target.type !== 'contract') return current;
      if (isDescendant(moving, targetId)) return current;
      const removed = filterNode(current, nodeId);
      return mapNode(removed, (node) => node.id === targetId
        ? { ...node, subContracts: [...(node.subContracts ?? []), moving] }
        : node,
      );
    });
  }, []);

  const validate = useCallback(() => {
    const result = validateAuthTree(root);
    setValidation(result);
  }, [root]);

  const exportPayload = useCallback(() => {
    const payload = generateAuthPayload(root);
    if (!payload) {
      setValidation(validateAuthTree(root));
      return;
    }
    setValidation({ valid: true, errors: [], payload });
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'soroban-auth-payload.json';
    anchor.click();
    URL.revokeObjectURL(url);
  }, [root]);

  const handleWheel = useCallback((event) => {
    event.preventDefault();
    const factor = event.deltaY > 0 ? 0.9 : 1.1;
    setZoom((value) => Math.min(4, Math.max(0.25, value * factor)));
  }, []);

  const handleCanvasMouseDown = useCallback((event) => {
    if (event.target.closest('[data-node]')) return;
    event.currentTarget.dataset.panStartX = String(event.clientX);
    event.currentTarget.dataset.panStartY = String(event.clientY);
    event.currentTarget.dataset.originX = String(pan.x);
    event.currentTarget.dataset.originY = String(pan.y);
  }, [pan]);

  const handleCanvasMouseMove = useCallback((event) => {
    const canvas = event.currentTarget;
    if (canvas.dataset.panStartX === undefined) return;
    const dx = event.clientX - Number(canvas.dataset.panStartX);
    const dy = event.clientY - Number(canvas.dataset.panStartY);
    setPan({
      x: Number(canvas.dataset.originX) + dx,
      y: Number(canvas.dataset.originY) + dy,
    });
  }, []);

  const stopPanning = useCallback((event) => {
    delete event.currentTarget.dataset.panStartX;
    delete event.currentTarget.dataset.panStartY;
  }, []);

  return (
    <main style={{ height: '100vh', display: 'flex', flexDirection: 'column', fontFamily: 'sans-serif', background: '#eef2f7' }}>
      <header style={{ display: 'flex', alignItems: 'center', gap: 8, padding: 10, borderBottom: '1px solid #d8dee9', background: '#fff' }}>
        <h2 style={{ margin: 0, fontSize: 18 }}>Soroban Authorization Builder</h2>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <button onClick={addContract} disabled={!selectedNode || selectedNode.type === 'address'}>+ Contract</button>
          <button onClick={addAddress} disabled={!selectedNode || selectedNode.type === 'address'}>+ Address</button>
          <button onClick={() => selectedId && removeNode(selectedId)} disabled={!selectedId}>Remove</button>
          <button onClick={() => setZoom((z) => Math.min(4, z * 1.15))}>Zoom +</button>
          <button onClick={() => setZoom((z) => Math.max(0.25, z * 0.87))}>Zoom -</button>
          <button onClick={validate}>Validate</button>
          <button onClick={exportPayload}>Export JSON</button>
        </div>
      </header>

      <div style={{ flex: 1, display: 'grid', gridTemplateColumns: '1fr 280px', minHeight: 0 }}>
        <div
          style={{ overflow: 'hidden', cursor: 'grab', position: 'relative' }}
          onWheel={handleWheel}
          onMouseDown={handleCanvasMouseDown}
          onMouseMove={handleCanvasMouseMove}
          onMouseUp={stopPanning}
          onMouseLeave={stopPanning}
        >
          <svg ref={svgRef} width="100%" height="100%">
            <g transform={`translate(${pan.x} ${pan.y}) scale(${zoom})`}>
              {renderLinks(nodes, positions).map((path, index) => (
                <path key={index} d={path} fill="none" stroke="#94a3b8" strokeWidth="2" />
              ))}
              {flattenNodes(nodes).map((node) => {
                const position = positions.get(node.id);
                if (!position) return null;
                const selected = node.id === selectedId;
                return (
                  <g
                    key={node.id}
                    data-node
                    transform={`translate(${position.x} ${position.y})`}
                    draggable
                    onDragStart={(event) => {
                      event.stopPropagation();
                      setDraggingNode(node.id);
                      event.dataTransfer.effectAllowed = 'move';
                    }}
                    onDragOver={(event) => {
                      if (draggingNode && node.type === 'contract') event.preventEvent?.();
                      if (node.type === 'contract') event.preventDefault();
                    }}
                    onDrop={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      if (draggingNode) moveNode(draggingNode, node.id);
                      setDraggingNode(null);
                    }}
                    onClick={(event) => {
                      event.stopPropagation();
                      setSelectedId(node.id);
                    }}
                    style={{ cursor: 'grab' }}
                  >
                    <rect
                      width={NODE_WIDTH}
                      height={NODE_HEIGHT}
                      rx={10}
                      fill={node.type === 'contract' ? '#2563eb' : '#059669'}
                      stroke={selected ? '#f59e0b' : '#fff'}
                      strokeWidth={selected ? 4 : 2}
                    />
                    <text x={12} y={21} fill="#fff" fontSize="13" fontWeight="700">{node.label}</text>
                    <text x={12} y={38} fill="#dbeafe" fontSize="9">
                      {(node.type === 'contract' ? node.contractId : node.address)?.slice(0, 22) ?? 'missing id'}
                    </text>
                  </g>
                );
              })}
            </g>
          </svg>
        </div>

        <aside style={{ borderLeft: '1px solid #d8dee9', background: '#fff', padding: 14, overflowY: 'auto' }}>
          <h3 style={{ marginTop: 0 }}>Node settings</h3>
          {selectedNode ? (
            <div style={{ display: 'grid', gap: 10 }}>
              <label>Label<input value={selectedNode.label} onChange={(event) => updateNode(selectedNode.id, { label: event.target.value })} /></label>
              <label>Type
                <select value={selectedNode.type} onChange={(event) => {
                  const type = event.target.value;
                  updateNode(selectedNode.id, type === 'contract'
                    ? { type, contractId: selectedNode.contractId ?? `C${'X'.repeat(55)}`, subContracts: selectedNode.subContracts ?? [] }
                    : { type, address: selectedNode.address ?? `G${'A'.repeat(55)}`, subContracts: undefined },
                  );
                }}>
                  <option value="contract">Contract</option>
                  <option value="address">Address</option>
                </select>
              </label>
              {selectedNode.type === 'contract' ? (
                <label>Contract ID<input value={selectedNode.contractId ?? ''} onChange={(event) => updateNode(selectedNode.id, { contractId: event.target.value })} /></label>
              ) : (
                <label>Address<input value={selectedNode.address ?? ''} onChange={(event) => updateNode(selectedNode.id, { address: event.target.value })} /></label>
              )}
            </div>
          ) : <p>No node selected.</p>}

          {validation && (
            <section style={{ marginTop: 18 }}>
              <h3>Validation</h3>
              <div style={{ color: validation.valid ? '#047857' : '#b91c1c', fontWeight: 700 }}>
                {validation.valid ? 'Valid authorization tree' : 'Invalid authorization tree'}
              </div>
              {validation.errors.length > 0 && (
                <ul>{validation.errors.map((error, index) => <li key={index}>{error}</li>)}</ul>
              )}
              {validation.payload && (
                <pre style={{ background: '#0f172a', color: '#e2e8f0', padding: 10, overflowX: 'auto', fontSize: 11 }}>
                  {JSON.stringify(validation.payload, null, 2)}
                </pre>
              )}
            </section>
          )}
        </aside>
      </div>
    </main>
  );
}

function mapNode(nodes, transform) {
  return nodes.map((node) => {
    const transformed = transform(node);
    if (transformed.subContracts) {
      return { ...transformed, subContracts: mapNode(transformed.subContracts, transform) };
    }
    return transformed;
  });
}

function filterNode(nodes, id) {
  return nodes
    .filter((node) => node.id !== id)
    .map((node) => ({
      ...node,
      subContracts: node.subContracts ? filterNode(node.subContracts, id) : undefined,
    }));
}

function findNode(nodes, id) {
  for (const node of nodes) {
    if (node.id === id) return node;
    const found = findNode(node.subContracts ?? [], id);
    if (found) return found;
  }
  return null;
}

function isDescendant(node, id) {
  return (node.subContracts ?? []).some((child) => child.id === id || isDescendant(child, id));
}

function flattenNodes(nodes) {
  return nodes.flatMap((node) => [node, ...flattenNodes(node.subContracts ?? [])]);
}

function layoutTree(nodes) {
  const positions = new Map();
  let leafIndex = 0;
  function walk(list, depth) {
    for (const node of list) {
      const children = node.subContracts ?? [];
      if (children.length === 0) {
        const y = 40 + leafIndex * VERTICAL_GAP;
        positions.set(node.id, { x: 40 + depth * HORIZONTAL_GAP, y });
        leafIndex += 1;
      } else {
        const childPositionsBefore = positions.size;
        walk(children, depth + 1);
        const childPositions = children.map((child) => positions.get(child.id)).filter(Boolean);
        const y = childPositions.length
          ? (childPositions[0].y + childPositions[childPositions.length - 1].y) / 2
          : 40;
        positions.set(node.id, { x: 40 + depth * HORIZONTAL_GAP, y });
      }
    }
  }
  walk(nodes, 0);
  return positions;
}

function renderLinks(nodes, positions) {
  const paths = [];
  function walk(list) {
    for (const node of list) {
      const from = positions.get(node.id);
      for (const child of node.subContracts ?? []) {
        const to = positions.get(child.id);
        if (from && to) {
          const startX = from.x + NODE_WIDTH;
          const startY = from.y + NODE_HEIGHT / 2;
          const endX = to.x;
          const endY = to.y + NODE_HEIGHT / 2;
          const midX = startX + (endX - startX) / 2;
          paths.push(`M ${startX} ${startY} C ${midX} ${startY}, ${midX} ${endY}, ${endX} ${endY}`);
        }
      }
      walk(node.subContracts ?? []);
    }
  }
  walk(nodes);
  return paths;
}
