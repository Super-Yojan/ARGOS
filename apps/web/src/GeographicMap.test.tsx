import { render,screen,fireEvent } from '@testing-library/react';
import { expect,it,vi } from 'vitest';
import GeographicMap from './GeographicMap';
import type { Rover } from './generated';
const observed=vi.hoisted(()=>({positions:[] as number[][],click:null as null|((e:unknown)=>void)}));
vi.mock('maplibre-gl',()=>({setWorkerUrl:vi.fn(),Marker:class {
 el:HTMLElement;constructor(o:{element:HTMLElement}){this.el=o.element}
 setLngLat(p:number[]){observed.positions.push(p);return this}setOffset(){return this}
 addTo(m:{container:HTMLElement}){m.container.append(this.el);return this}remove(){this.el.remove()}getElement(){return this.el}
},Map:class {
 container:HTMLElement;constructor(o:{container:HTMLElement}){this.container=o.container}
 on(name:string,callback:(e:unknown)=>void){if(name==='load')callback({});if(name==='click')observed.click=callback;return this}
 project(){return {x:300,y:200}}unproject(){return {lng:-77,lat:38}}addSource(){}addLayer(){}remove(){}fitBounds(){}getSource(){return {setData(){}}}
}}));
const rover={id:'1',pose:null,reference:null,gps:{latitude:38.8,longitude:-77.3,accuracy:24,age:0.2}} as Rover;
it('shows raw GPS without local pose, compass, geographic reference or a configured basemap',()=>{
 observed.positions=[];observed.click=null;
 const select=vi.fn(),draft=vi.fn();
 render(<GeographicMap rovers={[rover]} selected={rover} draft={null} styleURL={null} onDraft={draft} onLocal={vi.fn()} onSelect={select}/>);
 expect(screen.getByText('Fit fleet')).toBeEnabled();
 expect(screen.getByRole('button',{name:'Rover 1'})).toHaveTextContent('GPS ±24.0 m');
 expect(observed.positions.at(-1)).toEqual([-77.3,38.8]);
 fireEvent.click(screen.getByRole('button',{name:'Rover 1'}));expect(select).toHaveBeenCalledWith('1');
 (observed.click as ((e:unknown)=>void)|null)?.({lngLat:{lat:38,lng:-77}});expect(draft).not.toHaveBeenCalled();
 expect(screen.getByText(/Waypoint commands use the local map/)).toBeVisible();
});
it('starts displaying GPS when a fix arrives and removes stale or disconnected positions',()=>{
 const props={selected:undefined,draft:null,styleURL:null,onDraft:vi.fn(),onLocal:vi.fn()};
 const {rerender}=render(<GeographicMap {...props} rovers={[{...rover,gps:null}]}/>);
 expect(screen.getByText('Waiting for a fresh GPS fix.')).toBeVisible();
 rerender(<GeographicMap {...props} rovers={[rover]}/>);
 expect(screen.getByRole('button',{name:'Rover 1'})).toBeVisible();
 rerender(<GeographicMap {...props} rovers={[{...rover,gps:{...rover.gps!,age:4}}]}/>);
 expect(screen.queryByText('GPS ±24.0 m')).not.toBeInTheDocument();
 rerender(<GeographicMap {...props} rovers={[rover]} connected={false}/>);
 expect(screen.queryByText('GPS ±24.0 m')).not.toBeInTheDocument();
});
