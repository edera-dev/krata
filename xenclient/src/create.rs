use std::collections::HashMap;

pub struct DomainConfig {
    vm_entries: HashMap<String, String>,
    domain_entries: HashMap<String, String>,
}

pub struct PvDomainConfig {
    kernel: String,
    ramdisk: Option<String>,
    cmdline: Option<String>,
}

impl DomainConfig {
    pub fn new() -> DomainConfig {
        DomainConfig {
            vm_entries: HashMap::new(),
            domain_entries: HashMap::new(),
        }
    }

    pub fn put_vm(&mut self, key: &str, value: String) {
        self.vm_entries.insert(key.to_string(), value);
    }

    pub fn put_vm_str(&mut self, key: &str, value: &str) {
        self.put_vm(key, value.to_string());
    }

    pub fn put_domain(&mut self, key: &str, value: String) {
        self.vm_entries.insert(key.to_string(), value);
    }

    pub fn put_domain_str(&mut self, key: &str, value: &str) {
        self.put_domain(key, value.to_string());
    }

    pub fn configure_memory(&mut self, maxkb: u32, targetkb: u32, videokb: u32) {
        self.put_domain("memory/static-max", maxkb.to_string());
        self.put_domain("memory/target", targetkb.to_string());
        self.put_domain("memory/videoram", videokb.to_string());
    }

    pub fn configure_cpus(&mut self, maxvcpus: u32) {
        for i in 0..maxvcpus {
            println!("{}", i);
        }
    }

    pub fn configure_pv(&mut self, pv: PvDomainConfig) {
        self.put_vm_str("image/ostype", "linux");
        self.put_vm("image/kernel", pv.kernel);

        match pv.ramdisk {
            None => {}
            Some(ramdisk) => self.put_vm("image/ramdisk", ramdisk),
        }

        match pv.cmdline {
            None => {}
            Some(cmdline) => self.put_vm("image/cmdline", cmdline),
        }
    }

    pub fn clone_vm_entries(&self) -> HashMap<String, String> {
        self.vm_entries.clone()
    }

    pub fn clone_domain_entries(&self) -> HashMap<String, String> {
        self.domain_entries.clone()
    }
}

impl Default for DomainConfig {
    fn default() -> Self {
        DomainConfig::new()
    }
}

impl PvDomainConfig {
    pub fn new(kernel: String, ramdisk: Option<String>, cmdline: Option<String>) -> PvDomainConfig {
        PvDomainConfig {
            kernel,
            ramdisk,
            cmdline,
        }
    }
}
