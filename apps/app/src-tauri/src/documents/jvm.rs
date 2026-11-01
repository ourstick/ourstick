//! The calls into `Documents.kt` over JNI, from any of the core's threads
//! (ADR 0029). No `unsafe`: the Java VM and the class come from the
//! window's own JNI call at start, and every call runs in a local frame of
//! its own, so a thread that stays attached keeps no references.

use std::io;

use jni::objects::{
    GlobalRef, JByteArray, JClass, JLongArray, JObject, JObjectArray, JString, JValue,
};
use jni::{JNIEnv, JavaVM};

use super::Told;

/// An entry of a folder: its name, URI and facts.
pub struct Entry {
    pub name: String,
    pub uri: String,
    pub told: Told,
}

/// The Java VM and `Documents`.
pub struct Jvm {
    vm: JavaVM,
    documents: GlobalRef,
}

const CLASS: &str = "com/ourstick/OurStick/Documents";

impl Jvm {
    /// Taken on the window's thread, where the app's classes are found
    /// through the activity's class loader (a thread the core attached
    /// would not find them).
    pub fn new(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<Self> {
        let name = env.new_string(CLASS.replace('/', "."))?;
        let class = env
            .call_method(
                activity,
                "getAppClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::from(&name)],
            )?
            .l()?;
        Ok(Self {
            vm: env.get_java_vm()?,
            documents: env.new_global_ref(class)?,
        })
    }

    /// Runs `call` with this thread attached, in a local frame; an exception
    /// becomes an I/O error by its kind only (its message may hold a name).
    fn call<T>(
        &self,
        call: impl FnOnce(&mut JNIEnv, &JClass) -> jni::errors::Result<T>,
    ) -> io::Result<T> {
        let mut env = self
            .vm
            .attach_current_thread_permanently()
            .map_err(io::Error::other)?;
        let class = <&JClass>::from(self.documents.as_obj());
        // An exception is read and cleared inside the frame, so its
        // references go with it (the thread stays attached).
        let called = env.with_local_frame(32, |env| {
            Ok::<_, jni::errors::Error>(match call(env, class) {
                Ok(value) => Ok(value),
                Err(jni::errors::Error::JavaException) => Err(thrown(env)),
                Err(error) => Err(io::Error::other(error.to_string())),
            })
        });
        called.unwrap_or_else(|error| Err(io::Error::other(error.to_string())))
    }

    pub fn facts(&self, uri: &str) -> io::Result<Told> {
        self.call(|env, class| {
            let uri = env.new_string(uri)?;
            let array = env
                .call_static_method(
                    class,
                    "facts",
                    "(Ljava/lang/String;)[J",
                    &[JValue::from(&uri)],
                )?
                .l()?;
            told(env, JLongArray::from(array))
        })
    }

    pub fn entries(&self, uri: &str) -> io::Result<Vec<Entry>> {
        self.call(|env, class| {
            let uri = env.new_string(uri)?;
            let array = env
                .call_static_method(
                    class,
                    "entries",
                    "(Ljava/lang/String;)[Ljava/lang/String;",
                    &[JValue::from(&uri)],
                )?
                .l()?;
            let array = JObjectArray::from(array);
            let len = env.get_array_length(&array)?;
            let mut texts = Vec::with_capacity(len as usize);
            for index in 0..len {
                let text = JString::from(env.get_object_array_element(&array, index)?);
                texts.push(String::from(env.get_string(&text)?));
                env.delete_local_ref(text)?;
            }
            let mut entries = Vec::with_capacity(texts.len() / 5);
            let (rows, _) = texts.as_chunks::<5>();
            for row in rows {
                let number = |at: usize| row[at].parse::<i64>().unwrap_or(-1);
                entries.push(Entry {
                    name: row[0].clone(),
                    uri: row[1].clone(),
                    told: [number(2), number(3), number(4)],
                });
            }
            Ok(entries)
        })
    }

    /// How the place `a` relates to the place `b`: 0 apart, 1 the same, 2
    /// `a` holds `b`, 3 `a` lies inside `b`.
    pub fn relation(&self, a: &str, b: &str) -> io::Result<i32> {
        self.call(|env, class| {
            let a = env.new_string(a)?;
            let b = env.new_string(b)?;
            env.call_static_method(
                class,
                "relation",
                "(Ljava/lang/String;Ljava/lang/String;)I",
                &[JValue::from(&a), JValue::from(&b)],
            )?
            .i()
        })
    }

    /// Opens the file at `uri`; its handle in `Documents.kt`.
    pub fn open(&self, uri: &str) -> io::Result<i32> {
        self.call(|env, class| {
            let uri = env.new_string(uri)?;
            env.call_static_method(
                class,
                "open",
                "(Ljava/lang/String;)I",
                &[JValue::from(&uri)],
            )?
            .i()
        })
    }

    /// Fills `buffer` from `position` of the file `handle` as far as it can.
    pub fn read_at(&self, handle: i32, position: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let position = i64::try_from(position).map_err(io::Error::other)?;
        let len = i32::try_from(buffer.len()).unwrap_or(i32::MAX);
        self.call(|env, class| {
            let array: JByteArray = env.new_byte_array(len)?;
            let read = env
                .call_static_method(
                    class,
                    "readAt",
                    "(IJ[B)I",
                    &[
                        JValue::Int(handle),
                        JValue::Long(position),
                        JValue::from(&array),
                    ],
                )?
                .i()?;
            let read = usize::try_from(read).unwrap_or(0).min(buffer.len());
            let mut signed = vec![0i8; read];
            env.get_byte_array_region(&array, 0, &mut signed)?;
            for (byte, from) in buffer.iter_mut().zip(&signed) {
                *byte = *from as u8;
            }
            Ok(read)
        })
    }

    pub fn open_facts(&self, handle: i32) -> io::Result<Told> {
        self.call(|env, class| {
            let array = env
                .call_static_method(class, "openFacts", "(I)[J", &[JValue::Int(handle)])?
                .l()?;
            told(env, JLongArray::from(array))
        })
    }

    pub fn close(&self, handle: i32) -> io::Result<()> {
        self.call(|env, class| {
            env.call_static_method(class, "close", "(I)V", &[JValue::Int(handle)])?;
            Ok(())
        })
    }
}

fn told(env: &mut JNIEnv, array: JLongArray) -> jni::errors::Result<Told> {
    let mut told = [-1i64; 3];
    env.get_long_array_region(&array, 0, &mut told)?;
    Ok(told)
}

/// The pending exception, cleared, as an I/O error of its kind.
fn thrown(env: &mut JNIEnv) -> io::Error {
    let Ok(exception) = env.exception_occurred() else {
        return io::Error::other("Android refused");
    };
    let _ = env.exception_clear();
    let is = |env: &mut JNIEnv, class: &str| env.is_instance_of(&exception, class).unwrap_or(false);
    if is(env, "java/io/FileNotFoundException") {
        io::Error::new(io::ErrorKind::NotFound, "not found on this phone")
    } else if is(env, "java/lang/SecurityException") {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this phone no longer lets OurStick read it",
        )
    } else {
        io::Error::other("Android could not read it")
    }
}
