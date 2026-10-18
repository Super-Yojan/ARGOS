import type {Occupancy,Rover} from './generated';
import {scenePoint,type SceneFrame} from './fleetScene';
export function validOccupancy(g:Occupancy,r:Rover){
 return g.rover_id===r.id&&g.run_id===r.authority?.run_id&&g.frame_id===r.frame_id&&Number.isInteger(g.width)&&Number.isInteger(g.height)&&g.width>0&&g.height>0&&g.width<=4096&&g.height<=4096&&g.width*g.height<=250000&&Array.isArray(g.cells)&&g.cells.length===g.width*g.height&&g.cells.every(v=>Number.isInteger(v)&&v>=-1&&v<=100)&&Number.isFinite(g.resolution)&&g.resolution>0&&Number.isFinite(g.origin_x)&&Number.isFinite(g.origin_y)&&Number.isFinite(g.origin_x+g.width*g.resolution)&&Number.isFinite(g.origin_y+g.height*g.resolution)&&Number.isFinite(g.age)&&g.age>=0;
}
export function occupancyPixels(g:Occupancy){
 const pixels=new Uint8ClampedArray(g.cells.length*4);
 g.cells.forEach((v,i)=>{const shade=v<0?180:v>=65?0:255;pixels.set([shade,shade,shade,v<0?180:v>=65?220:235],i*4)});return pixels;
}
export function occupancyImage(g:Occupancy){
 const canvas=document.createElement('canvas');canvas.width=g.width;canvas.height=g.height;
 const context=canvas.getContext('2d');if(!context)return null;
 const image=context.createImageData(g.width,g.height);image.data.set(occupancyPixels(g));context.putImageData(image,0,0);return canvas.toDataURL('image/png');
}
export function occupancyCorners(g:Occupancy){return [{x:g.origin_x,y:g.origin_y},{x:g.origin_x+g.width*g.resolution,y:g.origin_y},{x:g.origin_x+g.width*g.resolution,y:g.origin_y+g.height*g.resolution},{x:g.origin_x,y:g.origin_y+g.height*g.resolution}]}
export function occupancyTransform(g:Occupancy,r:Rover,f:SceneFrame){
 const [a,b,,d]=occupancyCorners(g).map(p=>scenePoint(p,r,f));if(!a||!b||!d)return null;
 return [-(b.y-a.y)/g.width,-(b.x-a.x)/g.width,-(d.y-a.y)/g.height,-(d.x-a.x)/g.height,-a.y,-a.x].map(v=>v===0?0:v);
}
