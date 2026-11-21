use std::path::PathBuf;

#[cfg(windows)]
pub fn update(folders: Vec<PathBuf>, files: Vec<PathBuf>) {
    std::thread::spawn(move || {
        if let Err(e) = unsafe { build(&folders, &files) } {
            eprintln!("jump list: {e}");
        }
    });
}

#[cfg(not(windows))]
pub fn update(_folders: Vec<PathBuf>, _files: Vec<PathBuf>) {}

#[cfg(windows)]
fn quote(path: &std::path::Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(windows)]
unsafe fn build(folders: &[PathBuf], files: &[PathBuf]) -> windows::core::Result<()> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{SHStrDupW, DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let exe = HSTRING::from(std::env::current_exe().unwrap_or_default().as_os_str());
        let shell32 = HSTRING::from(r"%SystemRoot%\System32\shell32.dll");
        let link = |title: &str, args: &str, icon: Option<i32>| -> windows::core::Result<IShellLinkW> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&exe)?;
            link.SetArguments(&HSTRING::from(args))?;
            if let Some(index) = icon {
                link.SetIconLocation(&shell32, index)?;
            }
            let mut value = PROPVARIANT::default();
            (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
            (*value.Anonymous.Anonymous).Anonymous.pwszVal = SHStrDupW(&HSTRING::from(title))?;
            let store: IPropertyStore = link.cast()?;
            let set = store.SetValue(&PKEY_Title, &value);
            PropVariantClear(&mut value)?;
            set?;
            store.Commit()?;
            Ok(link)
        };
        let collection = || -> windows::core::Result<IObjectCollection> { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER) };

        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0;
        let removed: IObjectArray = list.BeginList(&mut slots)?;
        // re-adding an item the user removed from the list makes AppendCategory fail
        let mut gone = vec![];
        for i in 0..removed.GetCount()? {
            let mut args = [0u16; 1024];
            if let Ok(link) = removed.GetAt::<IShellLinkW>(i)
                && link.GetArguments(&mut args).is_ok()
            {
                gone.push(String::from_utf16_lossy(&args[..args.iter().position(|c| *c == 0).unwrap_or(0)]));
            }
        }
        let room = (slots as usize * 2 / 3).max(1);
        for (category, paths, take, icon) in [("Recent Folders", folders, room, 3), ("Recent", files, (slots as usize).saturating_sub(room).max(1), 0)] {
            let items = collection()?;
            let mut count = 0;
            for path in paths.iter().filter(|p| !gone.contains(&quote(p))).take(take) {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                items.AddObject(&link(&name, &quote(path), Some(icon))?)?;
                count += 1;
            }
            if count > 0 {
                list.AppendCategory(&HSTRING::from(category), &items.cast::<IObjectArray>()?)?;
            }
        }
        let tasks = collection()?;
        tasks.AddObject(&link("New Window", crate::windows::FLAG, None)?)?;
        list.AddUserTasks(&tasks.cast::<IObjectArray>()?)?;
        list.CommitList()
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn commits_recent_folders_files_and_the_new_window_task() {
        let dir = std::env::temp_dir();
        unsafe { super::build(std::slice::from_ref(&dir),&[dir.join("a.txt")]) }.unwrap();
        unsafe { super::build(&[], &[]) }.unwrap();
    }
}
