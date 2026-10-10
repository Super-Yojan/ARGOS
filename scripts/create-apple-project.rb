#!/usr/bin/env ruby
require 'xcodeproj'
root=File.expand_path('..',__dir__)
path=File.join(root,'apps/apple/ARGOS.xcodeproj')
project=Xcodeproj::Project.new(path)
shared=project.main_group.new_group('Shared','Shared')
source_refs=Dir[File.join(root,'apps/apple/Shared/*.swift')].sort.map { |file| shared.new_file(File.basename(file)) }
%w[Views Models Controllers Support].each do |folder|
 group=shared.new_group(folder,folder)
 source_refs += Dir[File.join(root,"apps/apple/Shared/#{folder}/*.swift")].sort.map { |file| group.new_file(File.basename(file)) }
end
generated=project.main_group.new_group('Generated','Generated')
swift=generated.new_file('ArgosCore.swift')
framework=generated.new_file('ArgosCore.xcframework')
framework.last_known_file_type='wrapper.xcframework'
resources=project.main_group.new_group('Resources','Resources')
resource_refs=Dir[File.join(root,'apps/apple/Resources/*')].sort.map { |file| resources.new_file(File.basename(file)) }
tests=project.main_group.new_group('Tests','Tests')
test_refs=Dir[File.join(root,'apps/apple/Tests/*.swift')].sort.map { |file| tests.new_file(File.basename(file)) }
[['ARGOSMac',:osx,'14.0'],['ARGOSiOS',:ios,'17.0']].each do |name,platform,version|
 app=project.new_target(:application,name,platform,version)
 (source_refs+[swift]).each { |ref| app.source_build_phase.add_file_reference(ref) }
 resource_refs.each { |ref| app.resources_build_phase.add_file_reference(ref) }
 app.frameworks_build_phase.add_file_reference(framework)
 %w[SwiftUI MapKit SceneKit GameController SystemConfiguration Security].each { |lib| app.add_system_framework(lib) }
 app.build_configurations.each do |config|
  config.build_settings.merge!({'PRODUCT_BUNDLE_IDENTIFIER'=>"org.argos.operator.#{platform}",'PRODUCT_MODULE_NAME'=>'ARGOS','SWIFT_VERSION'=>'5.0','CODE_SIGN_STYLE'=>'Automatic','GENERATE_INFOPLIST_FILE'=>'YES','MARKETING_VERSION'=>'0.1.0','CURRENT_PROJECT_VERSION'=>'1','INFOPLIST_KEY_CFBundleDisplayName'=>'ARGOS','INFOPLIST_FILE'=>'Support/Info.plist','INFOPLIST_KEY_NSLocalNetworkUsageDescription'=>'ARGOS connects to your Terra simulator or rover on the local network.','INFOPLIST_KEY_UILaunchScreen_Generation'=>'YES','INFOPLIST_KEY_UIApplicationSceneManifest_Generation'=>'YES','ENABLE_USER_SCRIPT_SANDBOXING'=>'YES'})
  if platform==:ios
   config.build_settings['TARGETED_DEVICE_FAMILY']='1,2'
   config.build_settings['SUPPORTED_PLATFORMS']='iphoneos iphonesimulator'
  else
   config.build_settings['CODE_SIGN_ENTITLEMENTS']='macOS/ARGOS.entitlements'
  end
 end
 unit=project.new_target(:unit_test_bundle,"#{name}Tests",platform,version)
 test_refs.each { |ref| unit.source_build_phase.add_file_reference(ref) }
 unit.add_dependency(app)
 unit.build_configurations.each do |config|
  executable=platform==:osx ? "#{name}.app/Contents/MacOS/#{name}" : "#{name}.app/#{name}"
  config.build_settings.merge!({'PRODUCT_BUNDLE_IDENTIFIER'=>"org.argos.operator.#{platform}.tests",'SWIFT_VERSION'=>'5.0','GENERATE_INFOPLIST_FILE'=>'YES','TEST_HOST'=>"$(BUILT_PRODUCTS_DIR)/#{executable}",'BUNDLE_LOADER'=>'$(TEST_HOST)'})
 end
 ui=project.new_target(:ui_test_bundle,"#{name}UITests",platform,version)
 ui_group=project.main_group.new_group("#{name} UI tests",'UITests')
 ui.source_build_phase.add_file_reference(ui_group.new_file('OperatorFlowTests.swift'))
 ui.add_dependency(app)
 ui.build_configurations.each do |config|
  config.build_settings.merge!({'PRODUCT_BUNDLE_IDENTIFIER'=>"org.argos.operator.#{platform}.uitests",'SWIFT_VERSION'=>'5.0','GENERATE_INFOPLIST_FILE'=>'YES','TEST_TARGET_NAME'=>name})
 end
 scheme=Xcodeproj::XCScheme.new

 scheme.add_build_target(app);scheme.set_launch_target(app);scheme.add_test_target(unit);scheme.add_test_target(ui)
 scheme.save_as(project.path,name,true)
end
project.save
