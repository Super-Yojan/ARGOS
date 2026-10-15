/** Screen-space callouts; offsets never change a rover's reported coordinates. */
export function markerOffsets(points:{id:string;x:number;y:number}[],width:number,height:number){
  const result:Record<string,{x:number;y:number}>={};
  const occupied:{x:number;y:number}[]=[];
  const candidates:{x:number;y:number}[]=[];
  const rings=Math.ceil(Math.sqrt(points.length))+2;
  for(let y=-rings;y<=rings;y++)for(let x=-rings;x<=rings;x++)candidates.push({x:x*150,y:y*90});
  candidates.sort((a,b)=>Math.hypot(a.x,a.y)-Math.hypot(b.x,b.y)||a.y-b.y||a.x-b.x);
  for(const point of points){
    const free=(offset:{x:number;y:number})=>!occupied.some(p=>Math.abs(p.x-point.x-offset.x)<150&&Math.abs(p.y-point.y-offset.y)<90);
    const fits=(o:{x:number;y:number})=>point.x+o.x>=75&&point.x+o.x<=width-75&&point.y+o.y>=35&&point.y+o.y<=height-65;
    const offset=candidates.find(o=>fits(o)&&free(o))??candidates.find(free)??{x:0,y:0};
    result[point.id]=offset;occupied.push({x:point.x+offset.x,y:point.y+offset.y});
  }
  return result;
}
