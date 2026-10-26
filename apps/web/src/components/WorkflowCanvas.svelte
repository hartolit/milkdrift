<script lang="ts">
  import type { CanvasState, GraphEdge, GraphNode, Point } from '../lib/canvas';
  let { canvas = $bindable(), nodes, edges, onselect, onconnect, ondropfile, onedge, onplace }: {
    canvas: CanvasState; nodes: GraphNode[]; edges: GraphEdge[];
    onplace: (point: Point) => void;
    onselect: (id: string) => void; onconnect: (source: string, target?: string, output?: string | null, input?: string | null) => void;
    ondropfile: (file: File, target: string) => void; onedge: (edge: GraphEdge) => void;
  } = $props();
  let surface: HTMLDivElement;
  let gesture: { kind: 'pan' | 'move' | 'wire'; id: string; origin: Point; start: Point; pointer: number; output: string | null } | null = null;
  let wire = $state<{ source: string; x: number; y: number } | null>(null);
  let suppressConnectorClick = false;
  let picked = $state<{ source: string; output: string | null } | null>(null);
  const nodeWidth = 230;
  const point = (event: PointerEvent): Point => ({ x: event.clientX, y: event.clientY });

  export function fit() {
    if (!surface || !nodes.length) return;
    const left = Math.min(...nodes.map((node) => node.position.x)), top = Math.min(...nodes.map((node) => node.position.y));
    const width = Math.max(...nodes.map((node) => node.position.x + nodeWidth)) - left;
    const height = Math.max(...nodes.map((node) => node.position.y + 150)) - top;
    const zoom = Math.max(.001, Math.min(1, (surface.clientWidth - 96) / width, (surface.clientHeight - 100) / height));
    canvas.camera = { x: (surface.clientWidth - width * zoom) / 2 - left * zoom, y: (surface.clientHeight - height * zoom) / 2 - top * zoom, zoom };
  }

  function zoom(delta: number) {
    const previous = canvas.camera.zoom, next = Math.max(.001, Math.min(1.75, previous + delta));
    const x = surface.clientWidth / 2, y = surface.clientHeight / 2;
    canvas.camera = { x: x - (x - canvas.camera.x) * next / previous, y: y - (y - canvas.camera.y) * next / previous, zoom: next };
  }

  function begin(event: PointerEvent, kind: 'pan' | 'move' | 'wire', id = '', output: string | null = null) {
    if (event.button !== 0) return;
    if (kind === 'pan' && event.target !== surface && !(event.target instanceof SVGElement)) return;
    const node = nodes.find((item) => item.id === id);
    gesture = { kind, id, output, pointer: event.pointerId, start: point(event), origin: kind === 'pan' ? { ...canvas.camera } : node?.position ?? { x: 0, y: 0 } };
    if (kind !== 'wire') surface.setPointerCapture(event.pointerId);
    if (kind !== 'pan') event.stopPropagation();
  }

  function move(event: PointerEvent) {
    if (!gesture || gesture.pointer !== event.pointerId) return;
    const dx = event.clientX - gesture.start.x, dy = event.clientY - gesture.start.y;
    if (gesture.kind === 'pan') canvas.camera = { ...canvas.camera, x: gesture.origin.x + dx, y: gesture.origin.y + dy };
    else if (gesture.kind === 'move') canvas.positions[gesture.id] = { x: Math.max(-2000, Math.min(12000, gesture.origin.x + dx / canvas.camera.zoom)), y: Math.max(-2000, Math.min(12000, gesture.origin.y + dy / canvas.camera.zoom)) };
    else {
      if (Math.hypot(dx, dy) < 5) return;
      if (!surface.hasPointerCapture(event.pointerId)) surface.setPointerCapture(event.pointerId);
      const bounds = surface.getBoundingClientRect();
      wire = { source: gesture.id, x: (event.clientX - bounds.left - canvas.camera.x) / canvas.camera.zoom, y: (event.clientY - bounds.top - canvas.camera.y) / canvas.camera.zoom };
    }
  }

  function end(event: PointerEvent) {
    if (!gesture || gesture.pointer !== event.pointerId) return;
    const completed = gesture;
    gesture = null; wire = null;
    if (surface.hasPointerCapture(event.pointerId)) surface.releasePointerCapture(event.pointerId);
    if (completed.kind === 'wire' && Math.hypot(event.clientX - completed.start.x, event.clientY - completed.start.y) > 5) {
      suppressConnectorClick = true;
      setTimeout(() => { suppressConnectorClick = false; }, 0);
      const port = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-input-node]');
      const target = port?.dataset.inputNode;
      if (target && target !== completed.id) { onconnect(completed.id, target, completed.output, port?.dataset.input || null); picked = null; }
    }
  }

  function path(edge: GraphEdge): string {
    const source = nodes.find((node) => node.id === edge.source), target = nodes.find((node) => node.id === edge.target);
    if (!source || !target) return '';
    const fromIndex = Math.max(0, source.outputs?.findIndex((port) => port.name === edge.output) ?? 0), toIndex = Math.max(0, target.inputs?.findIndex((port) => port.name === edge.input) ?? 0);
    const x1 = source.position.x + nodeWidth, y1 = source.position.y + 64 + fromIndex * 38, x2 = target.position.x, y2 = target.position.y + 64 + toIndex * 38;
    const bend = Math.max(55, Math.abs(x2 - x1) * .45);
    return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`;
  }
</script>

<div class="graph-shell">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (The canvas supports keyboard pan and zoom; every card and connection also has normal button/list controls.) -->
  <div class="graph-surface" bind:this={surface} role="region" aria-label="Workflow canvas" tabindex="0" ondragover={(event) => { if (event.dataTransfer?.types.includes('application/x-milkdrift-task') || event.dataTransfer?.types.includes('Files')) event.preventDefault(); }} ondrop={(event) => { if (event.target instanceof Element && event.target.closest('[data-node]')) return; event.preventDefault(); const bounds = surface.getBoundingClientRect(); if (event.dataTransfer?.getData('application/x-milkdrift-task')) onplace({ x: (event.clientX - bounds.left - canvas.camera.x) / canvas.camera.zoom, y: (event.clientY - bounds.top - canvas.camera.y) / canvas.camera.zoom }); else if (event.dataTransfer?.files[0]) ondropfile(event.dataTransfer.files[0], ''); }} onpointerdown={(event) => begin(event, 'pan')} onpointermove={move} onpointerup={end} onpointercancel={() => { gesture = null; wire = null; }} onwheel={(event) => { event.preventDefault(); if (event.ctrlKey || event.metaKey) zoom(event.deltaY < 0 ? .1 : -.1); else canvas.camera = { ...canvas.camera, x: canvas.camera.x - event.deltaX, y: canvas.camera.y - event.deltaY }; }} onkeydown={(event) => {
    if (event.key === 'Escape') { picked = null; return; }
    if (event.target !== surface) return;
    if (event.key === '+' || event.key === '=') zoom(.1);
    else if (event.key === '-') zoom(-.1);
    else if (event.key === 'Home') fit();
    else if (event.key.startsWith('Arrow')) canvas.camera = { ...canvas.camera, x: canvas.camera.x + (event.key === 'ArrowLeft' ? 80 : event.key === 'ArrowRight' ? -80 : 0), y: canvas.camera.y + (event.key === 'ArrowUp' ? 80 : event.key === 'ArrowDown' ? -80 : 0) };
    else return;
    event.preventDefault();
  }}>
    <div class="graph-plane" style={`transform:translate(${canvas.camera.x}px,${canvas.camera.y}px) scale(${canvas.camera.zoom})`}>
      <svg class="graph-wires" aria-hidden="true" width="1" height="1">
        <defs><marker id="connection-arrow" markerWidth="7" markerHeight="7" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6" fill="none" stroke="currentColor" /></marker></defs>
        {#each edges as edge (edge.id)}<path class:draft-wire={edge.draft} class:unfinished-wire={edge.unfinished} class:outcome-wire={edge.binding?.kind === 'settled_outcome'} d={path(edge)} marker-end="url(#connection-arrow)" />{/each}
        {#if wire}{@const source = nodes.find((node) => node.id === wire!.source)}{#if source}<path class="drawing-wire" d={`M${source.position.x + nodeWidth},${source.position.y + 64} L${wire.x},${wire.y}`} />{/if}{/if}
      </svg>
      {#each nodes as node (node.id)}
        <article class="graph-node {node.kind}" class:is-selected={canvas.selected === node.id && canvas.inspector} class:is-draft={node.draft} class:is-highlighted={canvas.highlighted.includes(node.id)} data-node={node.id} style={`left:${node.position.x}px;top:${node.position.y}px`} ondragover={(event) => { if (node.kind === 'task') event.preventDefault(); }} ondrop={(event) => { event.preventDefault(); const file = event.dataTransfer?.files[0]; if (file && node.kind === 'task') ondropfile(file, node.id); }}>
          <button class="node-move" aria-label={`Move ${node.label}`} title="Drag to arrange. Layout does not change execution." onpointerdown={(event) => begin(event, 'move', node.id)} onkeydown={(event) => {
            if (!event.key.startsWith('Arrow')) return; event.preventDefault();
            canvas.positions[node.id] = { x: node.position.x + (event.key === 'ArrowLeft' ? -30 : event.key === 'ArrowRight' ? 30 : 0), y: node.position.y + (event.key === 'ArrowUp' ? -30 : event.key === 'ArrowDown' ? 30 : 0) };
          }}>⠿</button>
          <button class="node-content" aria-label={`${node.label} · ${node.draft ? 'Draft' : node.status ?? node.state}`} aria-pressed={canvas.selected === node.id && canvas.inspector} onclick={() => onselect(node.id)}><span class="node-type">{node.kind === 'task' ? 'Task' : node.kind === 'boundary' ? 'Outside this page' : 'Input'}{#if node.draft}<span>Draft</span>{/if}</span><strong>{node.label}</strong>{#if node.model}<small class="node-model">{node.model}</small>{/if}<span class="node-description">{node.detail}</span><span class="node-state {node.state}"><span aria-hidden="true">●</span> {node.state === 'draft' ? 'Draft' : node.state === 'source' ? 'Ready to connect' : node.status ?? node.state}</span></button>
          {#if node.kind === 'task'}{#each node.inputs ?? [{ name: null, label: 'Input' }] as port, index (port.name ?? 'generic')}
            <button class="node-port input-port" style={`top:${42 + index * 38}px`} data-input-node={node.id} data-input={port.name ?? ''} aria-label={`${port.label} on ${node.label}`} title={picked ? `Connect to ${port.label}` : port.label} onclick={() => { if (picked && picked.source !== node.id) { onconnect(picked.source, node.id, picked.output, port.name); picked = null; } }}><span aria-hidden="true"></span><small>{port.label}</small></button>
          {/each}{/if}
          {#if node.kind !== 'boundary'}{#each node.outputs ?? [{ name: null, label: 'Output' }] as port, index (port.name ?? 'generic')}
            <button class="node-port output-port" style={`top:${42 + index * 38}px`} aria-label={`Connect ${port.label} from ${node.label}`} aria-pressed={picked?.source === node.id && picked.output === port.name} title={`Drag ${port.label} to an input, or select it then select an input`} onpointerdown={(event) => begin(event, 'wire', node.id, port.name)} onclick={() => { if (suppressConnectorClick) suppressConnectorClick = false; else picked = { source: node.id, output: port.name }; }}><span aria-hidden="true"></span><small>{port.label}</small></button>
          {/each}{/if}
        </article>
      {/each}
    </div>
  </div>
  <div class="camera-controls" aria-label="Canvas controls"><button aria-label="Zoom out" onclick={() => zoom(-.1)}>−</button><span>{(canvas.camera.zoom * 100).toLocaleString(undefined, { maximumFractionDigits: 1 })}%</span><button aria-label="Zoom in" onclick={() => zoom(.1)}>+</button><button onclick={fit}>Fit</button></div>
  <p class="canvas-hint" aria-live="polite">{picked ? `Choose an input for ${nodes.find((node) => node.id === picked!.source)?.label}. Escape cancels.` : 'Drag the background to pan · Join output and input circles'}</p>
  <details class="canvas-connections"><summary>Connections ({edges.length})</summary>{#each edges as edge (edge.id)}<button onclick={() => onedge(edge)}>{nodes.find((node) => node.id === edge.source)?.label} → {nodes.find((node) => node.id === edge.target)?.label}<small>{edge.label}{edge.unfinished ? ' · Unfinished' : ''}</small></button>{:else}<p class="field-hint">No connections. A task can work from instructions alone.</p>{/each}</details>
</div>
