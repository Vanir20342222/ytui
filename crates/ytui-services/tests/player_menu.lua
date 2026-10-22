-- Standalone Lua test for mpv window-menu behavior; synthetic mp API only.

local keys,properties,commands,events,messages={},{},{},{},{}
local mp={}
mp.msg={warn=function(message) messages[#messages+1]=message end}
function mp.create_osd_overlay() return {remove=function()end,update=function()end} end
function mp.get_osd_size() return 1920,1080 end
function mp.add_key_binding(_,name,fn) keys[name]=fn end
function mp.add_forced_key_binding(_,name,fn) keys[name]=fn end
function mp.remove_key_binding(name) keys[name]=nil end
function mp.register_script_message() end
function mp.register_event(name,fn) events[name]=fn end
function mp.observe_property() end
function mp.osd_message() end
function mp.set_property(name,value) properties[name]=value end
function mp.commandv(...) commands[#commands+1]={...} end
function mp.get_property_number() return 1.0 end
function mp.set_property_number(name,value) properties[name]=value end
package.preload['mp']=function()return mp end
package.preload['mp.msg']=function()return mp.msg end
package.preload['mp.assdraw']=function()return {ass_new=function()return {text='',append=function(self,text)self.text=self.text..text end}end}end
dofile('crates/ytui-services/assets/player-menu.lua')
events['file-loaded']()
events['end-file']({reason='quit'});assert(#messages==0)
events['end-file']({reason='eof'});assert(messages[1]=='YTUI_END_FILE:eof')
keys['ytui-player-menu']()
keys['ytui-menu-down'](); keys['ytui-menu-down'](); keys['ytui-menu-enter']()
assert(properties['user-data/ytui/action']=='quality:1080')
for _=1,3 do keys['ytui-menu-down']() end
keys['ytui-menu-enter'](); assert(commands[1][1]=='cycle' and commands[1][2]=='aid')
for _=1,7 do keys['ytui-menu-down']() end
keys['ytui-menu-enter'](); assert(properties['user-data/ytui/action']=='sponsor-toggle')
keys['ytui-menu-down'](); keys['ytui-menu-enter'](); assert(properties['user-data/ytui/action']=='sponsor-undo')
keys['ytui-menu-esc']();assert(keys['ytui-menu-enter']==nil)
print('Lua 5.4: menu syntax, navigation, quality/audio/SponsorBlock actions and close passed against a synthetic mp API')
