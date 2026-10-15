import { render,screen } from '@testing-library/react';
import { expect,it,vi } from 'vitest';
import GeographicMap from './GeographicMap';
import type { Rover } from './generated';
vi.mock('maplibre-gl',()=>({setWorkerUrl:vi.fn(),Marker:class {el:HTMLElement;constructor(o:{element:HTMLElement}){this.el=o.element}setLngLat(){return this}setOffset(){return this}addTo(){return this}remove(){}getElement(){return this.el}},Map:class {
  on(name:string,callback:()=>void){if(name==='load')callback();return this}
  project(){return {x:300,y:200}}unproject(){return {lng:-77,lat:38}}addSource(){} addLayer(){} remove(){} jumpTo(){} fitBounds(){} getLayer(){return undefined} getSource(){return {setData(){}}}
}}));
it('initializes the map when geographic alignment becomes available after the view opens',()=>{
  const r={id:'1',pose:{x:0,y:0,yaw:0},reference:null} as Rover;
  const props={rovers:[r],selected:r,draft:null,styleURL:'https://tiles.example/style.json',onDraft:vi.fn(),onLocal:vi.fn()};
  const {rerender}=render(<GeographicMap {...props}/>);expect(screen.getByText('Geographic map unavailable')).toBeVisible();
  const aligned={...r,reference:{latitude:38,longitude:-77,anchor_x:0,anchor_y:0,rotation:0,frame_id:'a'}};
  rerender(<GeographicMap {...props} rovers={[aligned]} selected={aligned}/>);
  expect(screen.getByText('Fit fleet')).toBeEnabled();expect(screen.queryByText('Geographic map unavailable')).not.toBeInTheDocument();
});
