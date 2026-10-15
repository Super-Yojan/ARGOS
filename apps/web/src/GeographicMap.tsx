import { useEffect, useMemo, useRef, useState } from 'react';
import { Map as MapLibreMap, setWorkerUrl, Marker, type ImageSource, type GeoJSONSource } from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
import workerURL from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
setWorkerUrl(workerURL);
import type { Occupancy, Rover, Waypoint } from './generated';
import { markerOffsets } from './markerLayout';
import { shortID } from './fleetScene';
import { geographic } from './geometry';
import {occupancyImage,occupancyCorners,validOccupancy} from './occupancy';
import styles from './App.module.css';
type Props = { occupancy?:Occupancy|null;occupancyStatus?:string; rovers: Rover[]; selected: Rover | undefined; draft: Waypoint | null; styleURL: string | null; onDraft: (w: Waypoint) => void; onLocal: () => void; onSelect?: (id: string) => void };
export default function GeographicMap({ rovers, selected, draft, styleURL, onDraft, onLocal, onSelect = () => { },occupancy=null,occupancyStatus='' }: Props) {
  const grid=occupancy&&selected&&validOccupancy(occupancy,selected)?occupancy:null;
  const raster=useMemo(()=>grid?occupancyImage(grid):null,[grid?.cells,grid?.width,grid?.height]);
  const [showOccupancy,setShowOccupancy]=useState(true);
  const imageURL=useRef('');
  const container = useRef<HTMLDivElement>(null), map = useRef<MapLibreMap | null>(null);
  const onDraftRef = useRef(onDraft); onDraftRef.current = onDraft;
  const onSelectRef = useRef(onSelect); onSelectRef.current = onSelect;
  const markers = useRef(new Map<string, Marker>());
  const fitted = useRef('');
  const [viewport, setViewport] = useState(0);
  const [loaded, setLoaded] = useState(false), [failed, setFailed] = useState(false);
  const initial = rovers.find(r => r.pose && r.reference)?.reference;
  useEffect(() => {
    setFailed(false);
    if (!styleURL || !initial || !container.current) return;
    try {
      const m = new MapLibreMap({ container: container.current, style: styleURL, center: [initial.longitude, initial.latitude], zoom: 18, attributionControl: { compact: false } });
      map.current = m;
      const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => { m.resize(); fitted.current = ''; setViewport(v => v + 1) });
      observer?.observe(container.current);
      m.on('load', () => {
        m.addSource('rovers', { type: 'geojson', data: { type: 'FeatureCollection', features: [] } });
        m.addLayer({id:'callout-connectors',type:'line',source:'rovers',filter:['==',['get','kind'],'connector'],paint:{'line-color':'#525252','line-width':1}});
        m.addLayer({ id: 'rover-marks', type: 'circle', source: 'rovers', filter: ['==', ['get', 'kind'], 'rover'], paint: { 'circle-radius': 6, 'circle-color': '#000000', 'circle-stroke-color': '#FFFFFF', 'circle-stroke-width': 1 } });
        m.addLayer({ id: 'goal-marks', type: 'circle', source: 'rovers', filter: ['!=', ['get', 'kind'], 'rover'], paint: { 'circle-radius': 9, 'circle-color': '#FFFFFF', 'circle-stroke-color': '#000000', 'circle-stroke-width': 1 } });
        setLoaded(true);
      });
      m.on('error', () => setFailed(true));
      m.on('move',()=>setViewport(v=>v+1));
      m.on('click', e => onDraftRef.current({ frame: 'geographic', latitude: Number(e.lngLat.lat.toFixed(7)), longitude: Number(e.lngLat.lng.toFixed(7)), yaw: null }));
      return () => { observer?.disconnect(); for (const marker of markers.current.values()) marker.remove(); markers.current.clear(); fitted.current = ''; m.remove(); map.current = null; setLoaded(false) };
    } catch { setFailed(true) }
  }, [styleURL, initial?.frame_id]);
  useEffect(() => {
    if (!loaded || !map.current) return;
    const visible = rovers.filter(r => r.pose && r.reference);
    const offsets=markerOffsets(visible.map(r=>{const p=geographic(r.pose!,r.reference!);const point=map.current!.project([p.longitude,p.latitude]);return {id:r.id,x:point.x,y:point.y}}),container.current?.clientWidth||1000,container.current?.clientHeight||600);
    for (const [id, marker] of markers.current) { if (!visible.some(r => r.id === id)) { marker.remove(); markers.current.delete(id) } }
    for (const r of visible) {
      const p = geographic(r.pose!, r.reference!);
      let marker = markers.current.get(r.id);
      if (!marker) { const button = document.createElement('button'); button.className = styles.geoRoverMarker; button.setAttribute('aria-label', `Rover ${r.id}`); button.onclick = e => { e.stopPropagation(); onSelectRef.current(r.id) }; marker = new Marker({ element: button }).setLngLat([p.longitude, p.latitude]).addTo(map.current); markers.current.set(r.id, marker) }
      marker.setLngLat([p.longitude,p.latitude]).setOffset([offsets[r.id].x,offsets[r.id].y]);
      const button=marker.getElement();const label=document.createElement('strong');label.textContent=`Rover ${shortID(r.id)}`;const status=document.createElement('small');status.textContent=r.hardware?.armed?'Armed':'Disarmed';button.replaceChildren(label,status);button.setAttribute('aria-pressed',String(selected?.id===r.id));
    }
    const signature = visible.map(r => r.id).join(',');
    if (signature && signature !== fitted.current) { fitted.current = signature; fitFleet() }
    const features: {type:'Feature';geometry:{type:'Point';coordinates:number[]}|{type:'LineString';coordinates:number[][]};properties:Record<string,string>}[] = [];
    for (const r of rovers) {
      if (r.pose && r.reference) { const p = geographic(r.pose, r.reference); features.push({ type: 'Feature', geometry: { type: 'Point', coordinates: [p.longitude, p.latitude] }, properties: { kind: 'rover', id: r.id } }) }
      if (r.goal && r.goal.state !== 'idle' && r.reference) { const p = geographic({ x: r.goal.x, y: r.goal.y }, r.reference); features.push({ type: 'Feature', geometry: { type: 'Point', coordinates: [p.longitude, p.latitude] }, properties: { kind: 'goal', id: r.id } }) }
    }
    for(const r of visible){const offset=offsets[r.id];if(!offset.x&&!offset.y)continue;const p=geographic(r.pose!,r.reference!);const point=map.current.project([p.longitude,p.latitude]);const end=map.current.unproject([point.x+offset.x,point.y+offset.y]);features.push({type:'Feature',geometry:{type:'LineString',coordinates:[[p.longitude,p.latitude],[end.lng,end.lat]]},properties:{kind:'connector'}})}
    if (draft?.frame === 'geographic') features.push({ type: 'Feature', geometry: { type: 'Point', coordinates: [draft.longitude, draft.latitude] }, properties: { kind: 'draft' } });
    (map.current.getSource('rovers') as GeoJSONSource).setData({ type: 'FeatureCollection', features });
  }, [rovers, draft, loaded, selected?.id, viewport]);
  useEffect(()=>{
    const m=map.current;if(!m||!loaded)return;
    const visible=showOccupancy&&!!grid&&!!raster&&!!selected?.reference;
    if(!visible){if(m.getLayer('occupancy-grid'))m.setLayoutProperty('occupancy-grid','visibility','none');return}
    const coordinates=occupancyCorners(grid!).map(p=>{const point=geographic(p,selected!.reference!);return [point.longitude,point.latitude] as [number,number]}) as [[number,number],[number,number],[number,number],[number,number]];
    let source=m.getSource('occupancy-grid') as ImageSource|undefined;
    if(!source){m.addSource('occupancy-grid',{type:'image',url:raster!,coordinates});m.addLayer({id:'occupancy-grid',type:'raster',source:'occupancy-grid',paint:{'raster-opacity':grid!.age>=2.5?0.45:1,'raster-resampling':'nearest','raster-fade-duration':0}},'callout-connectors');imageURL.current=raster!}
    else {if(imageURL.current!==raster){source.updateImage({url:raster!,coordinates});imageURL.current=raster!}else source.setCoordinates(coordinates);m.setLayoutProperty('occupancy-grid','visibility','visible');m.setPaintProperty('occupancy-grid','raster-opacity',grid!.age>=2.5?0.45:1)}
  },[loaded,raster,grid,selected?.reference,showOccupancy]);
  function fitFleet() { const positions = rovers.flatMap(r => r.pose && r.reference ? [geographic(r.pose, r.reference)] : []); if (!positions.length || !map.current) return; map.current.fitBounds([[Math.min(...positions.map(p => p.longitude)), Math.min(...positions.map(p => p.latitude))], [Math.max(...positions.map(p => p.longitude)), Math.max(...positions.map(p => p.latitude))]], { padding: 100, maxZoom: 19, duration: 0 }) }
  const missing = !styleURL || !initial;
  return <div className={styles.geoWrap}>
    <div className={styles.mapTools}><span className="yg-label">WGS84 · geographic map</span><button disabled={!loaded} onClick={fitFleet}>Fit fleet</button></div>
    <div className={styles.occupancyStatus}><label><input type="checkbox" checked={showOccupancy} onChange={e=>setShowOccupancy(e.target.checked)}/> Occupancy map</label><span>{grid?(selected?.reference?`${grid.age>=2.5?'Stale map':'Observed map'} · ${grid.width} × ${grid.height} · ${grid.age.toFixed(1)} s old`:'Occupancy needs geographic alignment; use the local map.'):occupancyStatus}</span></div>
    <div ref={container} className={styles.geoMap} />
    {(missing || failed) && <div className={styles.mapNotice} role="status"><strong>Geographic map unavailable</strong><p>{!styleURL ? 'No basemap is configured.' : !initial ? 'A usable rover geographic reference is required.' : 'Basemap could not be loaded.'} The local map remains available.</p><button onClick={onLocal}>Use local map</button></div>}
    <div className={styles.unlocated} aria-label="Vehicles without a shared position">{rovers.filter(r => !r.pose || !r.reference).length > 0 && <><span className="yg-label">Position unavailable</span><div>{rovers.filter(r => !r.pose || !r.reference).map(r => <button key={r.id} aria-label={`Rover ${r.id}`} onClick={() => onSelect(r.id)}><strong>Rover {shortID(r.id)}</strong><small>{r.pose ? 'Geographic alignment unavailable' : 'Position unknown'}</small></button>)}</div></>}</div>
    <p className={styles.occupancyLegend}>Occupancy: white free · black occupied · gray unknown. Selected rover only.</p>
    <p className={styles.mapCaption}>Reported positions and destinations. Map clicks draft a waypoint; confirmation sends it.</p>
  </div>
}
