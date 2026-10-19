import { test,expect } from '@playwright/test';
test.beforeEach(async({page})=>{await page.route('**/api/v1/config',route=>route.fulfill({json:{demo:true,tile_style:'https://maps.argos.test/style.json'}}));await page.route('https://maps.argos.test/style.json',route=>route.fulfill({json:{version:8,sources:{},layers:[{id:'paper',type:'background',paint:{'background-color':'#ffffff'}}]}}));await page.goto('/auth/dev');await expect(page.getByText('Stream connected',{exact:true})).toBeVisible()});
test('monitors unknown and disarmed rovers and uses a local map without tiles',async({page},testInfo)=>{
  await page.screenshot({path:testInfo.outputPath('dashboard.png'),fullPage:true});
  await expect(page.getByRole('group',{name:'Local rover map'})).toBeVisible();
  await expect(page.getByRole('navigation',{name:'Fleet vehicles'})).toHaveCount(0);await expect(page.getByRole('table')).toHaveCount(0);
  await expect(page.getByLabel('Rover details')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'Rover 1',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:'Rover 2',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Rover 18446744073709551615',exact:true}).click();await expect(page.getByLabel('Rover details')).toContainText('Unknown');
  await page.getByRole('button',{name:'Rover 2',exact:true}).click();await expect(page.getByLabel('Rover details')).toContainText('Disarmed');
  await page.getByRole('button',{name:'Geographic',exact:true}).click();await expect(page.getByText(/Reported GPS fixes with measured accuracy/)).toBeVisible();
  await page.getByRole('button',{name:'Local',exact:true}).click();await expect(page.getByRole('group',{name:'Local rover map'})).toBeVisible();
});
test('takes control, confirms a waypoint, observes arrival and cancels',async({page})=>{
  await page.getByRole('button',{name:'Rover 1',exact:true}).click();
  await page.getByRole('button',{name:'Take control'}).click();await expect(page.getByText('Held by this tab')).toBeVisible();
  await page.getByLabel('North / x (m)').fill('12');await page.getByLabel('West / y (m)').fill('3');
  await page.getByRole('button',{name:'Review waypoint'}).click();await expect(page.getByRole('region',{name:'Confirm waypoint'})).toBeVisible();
  await page.getByRole('button',{name:'Send waypoint',exact:true}).click();await expect(page.getByLabel('Rover details').getByRole('status')).toContainText('arrived',{timeout:15000});
  await page.getByRole('button',{name:'Cancel rover goal'}).click();await expect(page.getByLabel('Rover details').getByRole('status')).toContainText('cancelled');
  await page.getByRole('button',{name:'Release control'}).click();await expect(page.getByRole('button',{name:'Take control'})).toBeVisible();
});
test('another tab cannot take control and disconnect clears a draft',async({page,context})=>{
  await page.getByRole('button',{name:'Rover 1',exact:true}).click();
  await page.getByRole('button',{name:'Take control'}).click();await expect(page.getByText('Held by this tab')).toBeVisible();
  const second=await context.newPage();await second.goto('/');await expect(second.getByText('Stream connected',{exact:true})).toBeVisible();await expect(second.getByRole('button',{name:'Take control'})).toBeDisabled();
  await page.getByLabel('North / x (m)').fill('4');await page.getByLabel('West / y (m)').fill('1');await page.getByRole('button',{name:'Review waypoint'}).click();await expect(page.getByRole('region',{name:'Confirm waypoint'})).toBeVisible();
  await context.setOffline(true);await expect(page.getByText('Stream disconnected',{exact:true})).toBeVisible({timeout:10000});await expect(page.getByRole('region',{name:'Confirm waypoint'})).toHaveCount(0);await context.setOffline(false);
  await page.close();await second.close();
});
test('fits a narrow screen with keyboard-accessible coordinate controls',async({page})=>{
  await page.setViewportSize({width:390,height:844});await page.getByRole('button',{name:'Rover 1',exact:true}).click();await expect(page.getByLabel('North / x (m)')).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
});
test('raw GPS map keeps waypoint commands local and map clicks never draft orders',async({page})=>{
  await page.route('**/api/v1/config',route=>route.fulfill({json:{demo:true,tile_style:'https://maps.argos.test/style.json'}}));
  await page.route('https://maps.argos.test/style.json',route=>route.fulfill({json:{version:8,sources:{},layers:[{id:'paper',type:'background',paint:{'background-color':'#ffffff'}}]}}));
  let sent=0;page.on('request',request=>{if(request.method()==='POST'&&request.url().endsWith('/waypoints'))sent++});
  await page.reload();await expect(page.getByText('Stream connected',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Geographic',exact:true}).click();
  await expect(page.getByRole('button',{name:'Fit fleet',exact:true})).toBeEnabled({timeout:10000});
  await page.getByRole('button',{name:'Rover 1',exact:true}).click();
  await page.locator('.maplibregl-canvas').click({position:{x:150,y:200}});await expect(page.getByLabel('North / x (m)')).toHaveValue('');await expect(page.getByLabel('Latitude (°)')).toHaveCount(0);expect(sent).toBe(0);
});
test('coincident rovers remain distinct and individually selectable without a roster',async({page})=>{
  const response=await page.request.get('/api/v1/fleet');const snapshot=await response.json();
  snapshot.rovers[1].pose={...snapshot.rovers[0].pose};snapshot.rovers[1].reference={...snapshot.rovers[0].reference};
  await page.route('**/api/v1/events?*',route=>route.fulfill({contentType:'text/event-stream',body:`event: fleet\ndata: ${JSON.stringify(snapshot)}\n\n`}));
  await page.reload();const first=page.getByRole('button',{name:'Rover 1',exact:true}),second=page.getByRole('button',{name:'Rover 2',exact:true});
  await expect(first).toBeVisible();await expect(second).toBeVisible();
  const a=await first.boundingBox(),b=await second.boundingBox();expect(a&&b&&(a.x+a.width<=b.x||b.x+b.width<=a.x||a.y+a.height<=b.y||b.y+b.height<=a.y)).toBe(true);
  await first.click();await expect(page.getByLabel('Rover details').getByRole('heading',{name:'Rover 1',exact:true})).toBeVisible();
  await second.click();await expect(page.getByLabel('Rover details').getByRole('heading',{name:'Rover 2',exact:true})).toBeVisible();
});
test('arms a disarmed demo rover only after confirmation, then disarms it',async({page},testInfo)=>{
  await page.getByRole('button',{name:'Rover 2',exact:true}).click();
  await expect(page.getByRole('button',{name:'Arm vehicle',exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'Take control'}).click();
  await expect(page.getByRole('button',{name:'Arm vehicle',exact:true})).toBeEnabled();
  await page.getByRole('button',{name:'Arm vehicle',exact:true}).click();
  await expect(page.getByRole('region',{name:'Confirm arming'})).toBeVisible();
  await page.screenshot({path:testInfo.outputPath('arm-confirmation.png'),fullPage:true});
  await page.getByRole('button',{name:'Confirm arm rover 2',exact:true}).click();
  await expect(page.getByLabel('Rover details').getByRole('status')).toContainText('Rover confirms armed');
  await page.getByRole('button',{name:'Disarm vehicle',exact:true}).click();
  await expect(page.getByLabel('Rover details').getByRole('status')).toContainText('Rover confirms disarmed');
  await page.getByRole('button',{name:'Release control'}).click();
});
test('selects direct waypoint autonomy and waits for the rover acknowledgement',async({page},testInfo)=>{
 await page.getByRole('button',{name:'Rover 2',exact:true}).click();await page.getByRole('button',{name:'Take control'}).click();
 await page.getByRole('button',{name:'L2 · Waypoint · no avoidance',exact:true}).click();
 await expect(page.getByText('Vehicle acknowledged waypoint_direct',{exact:true})).toBeVisible();
 await expect(page.getByRole('button',{name:'L2 · Waypoint · no avoidance',exact:true})).toHaveAttribute('aria-pressed','true');
 await expect(page.getByLabel('Rover details')).toContainText('Vehicle is disarmed');
 await expect(page.getByRole('button',{name:'L5 · Decision making',exact:true})).toBeDisabled();
 await page.getByRole('button',{name:'L3 · Waypoint · obstacle aware',exact:true}).scrollIntoViewIfNeeded();
 await page.screenshot({path:testInfo.outputPath('autonomy-levels.png'),fullPage:true});
 await page.getByRole('button',{name:'Release control'}).click();
});
test('renders occupancy below every rover and labels stale or incompatible maps',async({page},testInfo)=>{
 await page.getByRole('button',{name:'Rover 1',exact:true}).click();
 const image=page.locator('image[aria-label="Observed occupancy grid"]');
 await expect(image).toBeVisible();await expect(page.getByText(/Observed map · 40 × 40/)).toBeVisible();
 await page.getByRole('button',{name:'Fit fleet',exact:true}).click();
 await expect(page.getByRole('button',{name:'Rover 2',exact:true})).toBeVisible();
 await page.screenshot({path:testInfo.outputPath('occupancy-grid.png'),fullPage:true});
 await page.getByRole('checkbox',{name:'Occupancy map',exact:true}).uncheck();await expect(image).toHaveCount(0);
 await page.getByRole('checkbox',{name:'Occupancy map',exact:true}).check();await expect(image).toBeVisible();
 const grid=await (await page.request.get('/api/v1/rovers/1/occupancy')).json();
 await page.route('**/api/v1/rovers/1/occupancy',route=>route.fulfill({json:{...grid,age:8}}));
 await expect(page.getByText(/Stale map · 40 × 40/)).toBeVisible();await expect(image).toHaveAttribute('opacity','0.45');
 await page.unroute('**/api/v1/rovers/1/occupancy');
 await page.route('**/api/v1/rovers/1/occupancy',route=>route.fulfill({json:{...grid,run_id:'old-run'}}));
 await expect(page.getByText(/Occupancy grid does not match/)).toBeVisible();await expect(image).toHaveCount(0);
});
test('quick buttons apply vehicle geometry and avoidance settings only to a disarmed rover',async({page},testInfo)=>{
 await page.getByRole('button',{name:'Rover 2',exact:true}).click();await page.getByRole('button',{name:'Take control'}).click();
 await page.getByText('Dimensions & avoidance',{exact:true}).click();
 await expect(page.getByRole('button',{name:'Increase length'})).toBeEnabled();
 await page.getByRole('button',{name:'Increase length'}).click();
 await expect(page.getByLabel('Length (cm)')).toHaveValue('90');
 await page.getByRole('button',{name:'Apply settings',exact:true}).click();
 await expect(page.getByText('Terra applied vehicle dimensions and avoidance settings',{exact:true})).toBeVisible();
 const state=await (await page.request.get('/api/v1/fleet')).json();const rover=state.rovers.find((r:{id:string})=>r.id==='2');expect(rover.navigation.parameters.length).toBe(0.9);expect(rover.hardware.armed).toBe(false);
 await page.getByLabel('Clearance (cm)').scrollIntoViewIfNeeded();
 await page.screenshot({path:testInfo.outputPath('navigation-settings.png'),fullPage:true});
 await page.getByRole('button',{name:'Release control'}).click();
});

test('manual takeover drives only while held and emergency stop disarms',async({page})=>{
 await page.getByRole('button',{name:'Rover 1',exact:true}).click();
 await page.getByRole('button',{name:'Take control',exact:true}).click();
 await page.getByRole('button',{name:'Take over manual driving',exact:true}).click();
 await expect(page.getByRole('button',{name:'Forward',exact:true})).toBeEnabled();
 const moving=page.waitForResponse(r=>r.url().endsWith('/rovers/1/drive')&&r.request().postDataJSON()?.linear===0.3&&r.ok());
 await page.keyboard.down('w'); await moving;
 const neutral=page.waitForResponse(r=>r.url().endsWith('/rovers/1/drive')&&r.request().postDataJSON()?.linear===0&&r.request().postDataJSON()?.angular===0&&r.ok());
 await page.keyboard.up('w'); await neutral;
 await page.getByRole('button',{name:'Emergency stop',exact:true}).click();
 await expect.poll(async()=>{const s=await(await page.request.get('/api/v1/fleet')).json();const r=s.rovers.find((r:{id:string})=>r.id==='1');return {armed:r.hardware.armed,safety:r.authority.safety,claim:s.claim}}).toEqual({armed:false,safety:'stop',claim:null});
 await expect(page.getByRole('button',{name:'Forward',exact:true})).toBeDisabled();
});
