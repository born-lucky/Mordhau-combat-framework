$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, System.Windows.Forms
$tool = $env:MH_SETUP_TOOL
$root = $env:MH_SETUP_ROOT
[xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 Title="Mordhau Combat Framework" Width="1040" Height="720" MinWidth="960" MinHeight="680" WindowStartupLocation="CenterScreen"
 Background="#203746" Foreground="#EBE9DB" FontFamily="Segoe UI" ResizeMode="CanMinimize">
 <Window.Resources>
  <Style TargetType="Button">
   <Setter Property="Background" Value="#D3BC78"/><Setter Property="Foreground" Value="#203746"/>
   <Setter Property="BorderThickness" Value="0"/><Setter Property="Padding" Value="16,10"/>
   <Setter Property="FontSize" Value="14"/><Setter Property="FontWeight" Value="SemiBold"/>
   <Setter Property="Cursor" Value="Hand"/><Setter Property="Margin" Value="0,0,10,0"/>
   <Style.Triggers><Trigger Property="IsEnabled" Value="False"><Setter Property="Opacity" Value="0.36"/></Trigger></Style.Triggers>
  </Style>
  <Style TargetType="TextBox">
   <Setter Property="Background" Value="#142A39"/><Setter Property="Foreground" Value="#EBE9DB"/>
   <Setter Property="BorderBrush" Value="#68818B"/><Setter Property="Padding" Value="11,10"/>
   <Setter Property="FontSize" Value="14"/><Setter Property="VerticalContentAlignment" Value="Center"/>
  </Style>
 </Window.Resources>
 <Grid Background="#203746" TextElement.Foreground="#EBE9DB" TextElement.FontFamily="Segoe UI">
  <Grid.ColumnDefinitions><ColumnDefinition Width="258"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
  <Border Background="#142A39" Padding="28,32,24,24">
   <Grid>
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <StackPanel>
     <Canvas Width="175" Height="108" HorizontalAlignment="Left" Margin="0,0,0,20">
      <Path Stroke="#D3BC78" StrokeThickness="2" Data="M 16,94 L 143,7 L 137,30 L 41,96 Z"/>
      <Path Stroke="#A7C3CD" StrokeThickness="1.5" Data="M 0,94 Q 31,45 118,26 M 0,94 Q 66,91 145,36"/>
      <Line Stroke="#EBE9DB" StrokeThickness="5" X1="12" Y1="98" X2="33" Y2="82"/>
      <Line Stroke="#D3BC78" StrokeThickness="3" X1="18" Y1="77" X2="46" Y2="106"/>
     </Canvas>
     <TextBlock Text="Mordhau" FontSize="30" FontWeight="SemiBold"/>
     <TextBlock Text="Combat Framework" FontSize="20" Margin="0,4,0,0"/>
     <TextBlock Text="Rust / Bevy" FontSize="14" Foreground="#A7C3CD" Margin="0,12,0,26"/>
    </StackPanel>
    <StackPanel Grid.Row="1">
     <TextBlock Text="Precision in motion." FontSize="17" Foreground="#D3BC78" Margin="0,0,0,12"/>
     <TextBlock Text="Attacks follow the posed weapon. Timing, collision and presentation stay separate and inspectable." TextWrapping="Wrap" FontSize="14" LineHeight="22" Foreground="#A7C3CD"/>
    </StackPanel>
    <StackPanel Grid.Row="3">
     <TextBlock Text="Built in appreciation of Triternion." TextWrapping="Wrap" FontSize="13" Foreground="#A7C3CD" Margin="0,0,0,12"/>
     <Button x:Name="Buy" Content="Support the original game" Background="#284657" Foreground="#EBE9DB" FontSize="12" Padding="12,10"/>
     <TextBlock Text="Unofficial fan project. Use a purchased MORDHAU installation." TextWrapping="Wrap" FontSize="11" LineHeight="16" Foreground="#819EAB" Margin="0,12,0,0"/>
    </StackPanel>
   </Grid>
  </Border>
  <Grid Grid.Column="1" Margin="34,30,34,24">
   <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
   <StackPanel>
    <TextBlock Text="Set up the combat framework" FontSize="27" FontWeight="SemiBold"/>
    <TextBlock Text="Your original game stays installed. Its data is prepared locally." FontSize="14" Foreground="#A7C3CD" Margin="0,10,0,20"/>
   </StackPanel>
   <StackPanel Grid.Row="1">
    <TextBlock Text="1. Locate MORDHAU" FontSize="17" Margin="0,0,0,12"/>
    <Grid>
     <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="102"/></Grid.ColumnDefinitions>
     <TextBox x:Name="GamePath" AutomationProperties.Name="Original MORDHAU installation folder"/>
     <Button x:Name="Browse" Grid.Column="1" Content="Browse" Margin="10,0,0,0" Padding="12,10"/>
    </Grid>
    <TextBlock Text="Choose the Steam MORDHAU folder or its original executable." Foreground="#A7C3CD" FontSize="12" Margin="0,8,0,14"/>
    <StackPanel Orientation="Horizontal"><Button x:Name="Check" Content="Check installation"/><TextBlock x:Name="InstallState" Text="Waiting for a game folder" VerticalAlignment="Center" Foreground="#A7C3CD" FontSize="13" TextWrapping="Wrap" MaxWidth="340"/></StackPanel>
   </StackPanel>
   <StackPanel Grid.Row="2" Margin="0,24,0,0">
    <TextBlock Text="2. Prepare your local data" FontSize="17" Margin="0,0,0,10"/>
    <TextBlock Text="Read combat records and assets from your installation. Game files and generated data are never included in the download." Foreground="#A7C3CD" FontSize="13" LineHeight="20" TextWrapping="Wrap" Margin="0,0,0,12"/>
    <StackPanel Orientation="Horizontal"><Button x:Name="Prepare" Content="Prepare local files" IsEnabled="False"/><TextBlock x:Name="DataState" Text="Installation check required" VerticalAlignment="Center" Foreground="#A7C3CD" FontSize="13"/></StackPanel>
   </StackPanel>
   <Border Grid.Row="3" Background="#192F3E" Margin="0,22,0,18" Padding="18,15">
    <Grid>
     <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
     <TextBlock Text="Setup activity" FontSize="14" Foreground="#D3BC78" Margin="0,0,0,9"/>
     <ScrollViewer Grid.Row="1" VerticalScrollBarVisibility="Auto">
      <TextBlock x:Name="Activity" Text="This preview validates your installation. The complete importer and playable package are still in development." TextWrapping="Wrap" FontSize="13" LineHeight="20" Foreground="#A7C3CD"/>
     </ScrollViewer>
    </Grid>
   </Border>
   <Grid Grid.Row="4">
    <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
    <StackPanel>
     <TextBlock Text="3. Enter the combat lab" FontSize="17"/>
     <TextBlock x:Name="LaunchState" Text="Launch unlocks when local preparation is complete." Foreground="#A7C3CD" FontSize="12" Margin="0,7,0,0"/>
    </StackPanel>
    <Button x:Name="Launch" Grid.Column="1" Content="Launch sandbox" IsEnabled="False" Margin="14,0,0,0" VerticalAlignment="Center"/>
   </Grid>
  </Grid>
 </Grid>
</Window>
'@
$reader = New-Object System.Xml.XmlNodeReader $xaml
$window = [Windows.Markup.XamlReader]::Load($reader)
$controls = @{}
'Buy','GamePath','Browse','Check','Prepare','Launch','InstallState','DataState','LaunchState','Activity' | ForEach-Object { $controls[$_] = $window.FindName($_) }
$controls.GamePath.Text = if ($env:MORDHAU_DIR) { $env:MORDHAU_DIR } else { 'C:\Program Files (x86)\Steam\steamapps\common\Mordhau' }
$script:verifiedPath = ''
$script:busy = $false
$script:job = $null
$script:mode = ''

function Reset-Readiness {
 $script:verifiedPath = ''
 $controls.Prepare.IsEnabled = $false
 $controls.Launch.IsEnabled = $false
 $controls.InstallState.Text = 'Waiting for an installation check'
 $controls.InstallState.Foreground = [Windows.Media.BrushConverter]::new().ConvertFrom('#A7C3CD')
 $controls.DataState.Text = 'Installation check required'
 $controls.LaunchState.Text = 'Launch unlocks when local preparation is complete.'
}
$controls.GamePath.Add_TextChanged({ Reset-Readiness })
$controls.Buy.Add_Click({ Start-Process 'https://store.steampowered.com/app/629760/MORDHAU/' })
$controls.Browse.Add_Click({
 $dialog = New-Object System.Windows.Forms.OpenFileDialog
 $dialog.Title = 'Choose your original MORDHAU executable'
 $dialog.Filter = 'MORDHAU executable|Mordhau.exe;Mordhau-Win64-Shipping.exe'
 if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  $directory = [System.IO.Path]::GetDirectoryName($dialog.FileName)
  if ([System.IO.Path]::GetFileName($dialog.FileName) -eq 'Mordhau-Win64-Shipping.exe') {
   1..3 | ForEach-Object { $directory = [System.IO.Path]::GetDirectoryName($directory) }
  }
  $controls.GamePath.Text = $directory
 }
})

function Begin-Operation([string]$mode) {
 if ($script:busy) { return }
 $script:busy = $true; $script:mode = $mode
 $controls.Check.IsEnabled = $false; $controls.Prepare.IsEnabled = $false; $controls.Launch.IsEnabled = $false
 $controls.Browse.IsEnabled = $false; $controls.GamePath.IsEnabled = $false
 $controls.Activity.Text = switch ($mode) {
  '--check' { 'Checking the original executable, supported build, paks and physics libraries...' }
  '--prepare' { 'Preparing local files from your installation. The original files stay untouched.' }
  '--launch' { 'Checking the installation and local data again before launching...' }
 }
 $argv = @($mode,'--game-dir',$controls.GamePath.Text,'--root',$root,'--json')
 $script:job = Start-Job -ScriptBlock { param($exe,$arguments) & $exe @arguments } -ArgumentList $tool, $argv
 $timer.Start()
}
$timer = New-Object System.Windows.Threading.DispatcherTimer
$timer.Interval = [TimeSpan]::FromMilliseconds(150)
$timer.Add_Tick({
 if ($script:job.State -notin @('Completed','Failed','Stopped')) { return }
 $timer.Stop()
 try {
  $raw = (Receive-Job -Job $script:job -ErrorAction Stop) -join [Environment]::NewLine
  $result = $raw | ConvertFrom-Json -ErrorAction Stop
  if ($result.launch_requested) {
   $controls.Activity.Text = 'Launch requested. The game window is starting; gameplay readiness has not been confirmed.'
  } else {
   $controls.InstallState.Text = if ($result.installed) { 'Local runtime files found' } else { 'Required runtime files unavailable' }
   $controls.InstallState.Foreground = if ($result.installed) { [Windows.Media.BrushConverter]::new().ConvertFrom('#92C5B5') } else { [Windows.Media.BrushConverter]::new().ConvertFrom('#D8A584') }
   $controls.DataState.Text = if ($result.local_data_ready) { 'Local data is ready' } else { 'Local preparation is incomplete' }
   $controls.LaunchState.Text = if ($result.ready) { 'Ready to launch.' } else { 'Launch remains locked until setup is complete.' }
   $controls.Prepare.IsEnabled = [bool]$result.installed
   $controls.Launch.IsEnabled = [bool]$result.ready
   $controls.Activity.Text = if ($result.error) { [string]$result.error } else { 'Installation and local files passed validation.' }
   if ($result.installed) { $script:verifiedPath = $controls.GamePath.Text }
  }
 } catch {
  $controls.Activity.Text = 'Setup did not complete: ' + $_.Exception.Message
  Reset-Readiness
 } finally {
  Remove-Job -Job $script:job -Force
  $script:job = $null
  $script:busy = $false
  $controls.Check.IsEnabled = $true; $controls.Browse.IsEnabled = $true; $controls.GamePath.IsEnabled = $true
 }
})
$controls.Check.Add_Click({ Begin-Operation '--check' })
$controls.Prepare.Add_Click({ Begin-Operation '--prepare' })
$controls.Launch.Add_Click({ Begin-Operation '--launch' })
$window.Add_Closing({
 param($sender,$event)
 if ($script:busy) {
  $event.Cancel = $true
  $controls.Activity.Text += [Environment]::NewLine + 'Wait for the current operation to finish before closing setup.'
  return
 }
 $timer.Stop()
 if ($script:job) { Stop-Job -Job $script:job; Remove-Job -Job $script:job -Force }
})

if ($env:MH_SETUP_PREVIEW) {
 # Render this application's layout directly; no desktop capture or game assets are used.
 $content = $window.Content
 $window.Content = $null
 $content.Resources = $window.Resources
 $content.Measure([Windows.Size]::new(1040,720))
 $content.Arrange([Windows.Rect]::new(0,0,1040,720))
 $content.UpdateLayout()
 $bitmap = [Windows.Media.Imaging.RenderTargetBitmap]::new(1040,720,96,96,[Windows.Media.PixelFormats]::Pbgra32)
 $bitmap.Render($content)
 $encoder = [Windows.Media.Imaging.PngBitmapEncoder]::new()
 $encoder.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bitmap))
 $file = [System.IO.File]::Create($env:MH_SETUP_PREVIEW)
 try { $encoder.Save($file) } finally { $file.Dispose() }
} else {
 [void]$window.ShowDialog()
}
