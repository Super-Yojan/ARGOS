import {expect,it} from 'vitest';
import {markerOffsets} from './markerLayout';
it('keeps coincident vehicles individually visible without changing their reported positions',()=>{
 const points=[{id:'1',x:200,y:200},{id:'2',x:200,y:200},{id:'3',x:201,y:201}];const result=markerOffsets(points,600,500);
 const positions=points.map(p=>({x:p.x+result[p.id].x,y:p.y+result[p.id].y}));
 for(let i=0;i<positions.length;i++)for(let j=i+1;j<positions.length;j++)expect(Math.abs(positions[i].x-positions[j].x)>=150||Math.abs(positions[i].y-positions[j].y)>=90).toBe(true);
 expect(points[1]).toEqual({id:'2',x:200,y:200});
});
it('keeps separated positions stable and fits callouts on a narrow viewport',()=>{
 const points=[{id:'1',x:190,y:200},{id:'2',x:190,y:200}];const result=markerOffsets(points,390,500);
 for(const p of points){expect(p.x+result[p.id].x).toBeGreaterThanOrEqual(75);expect(p.x+result[p.id].x).toBeLessThanOrEqual(315)}
 expect(markerOffsets([{id:'a',x:150,y:150},{id:'b',x:500,y:350}],700,500)).toEqual({a:{x:0,y:0},b:{x:0,y:0}});
});
