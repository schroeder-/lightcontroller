#[macro_use]
extern crate rocket;

use rocket::fs::{relative, FileServer};
use rocket::http::Status;
use rocket::log::warn_;
use rocket::serde::json::{serde_json, Json};
use rocket::tokio::io::AsyncWriteExt;
use rocket::tokio::sync::{Mutex, RwLock};
use rocket::State;
use rocket_dyn_templates::Template;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::convert::TryFrom;
use std::fs::File;
use std::sync::Arc;

static QOS: QoS = QoS::AtLeastOnce;

#[derive(Serialize_repr, Deserialize_repr, PartialEq, Debug, Clone)]
#[repr(u8)]
enum LightModes {
    Off = 0,
    Rainbow = 1,
    RainbowSlow = 2,
    Mood = 3,
    Blue = 4,
    BlueMood = 5,
    Flame = 6,
    White = 7,
    Color = 8,
    Orange = 9,
    None = 255,
}

impl TryFrom<String> for LightModes {
    type Error = ();

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Ok(v) = value.parse::<u32>() {
            match v {
                0 => Ok(LightModes::Off),
                1 => Ok(LightModes::Rainbow),
                2 => Ok(LightModes::RainbowSlow),
                3 => Ok(LightModes::Mood),
                4 => Ok(LightModes::Blue),
                5 => Ok(LightModes::BlueMood),
                6 => Ok(LightModes::Flame),
                7 => Ok(LightModes::White),
                8 => Ok(LightModes::Color),
                9 => Ok(LightModes::Orange),
                _ => Err(()),
            }
        } else {
            Err(())
        }
    }
}

impl From<u32> for LightModes {
    fn from(value: u32) -> Self {
        match value {
            0 => LightModes::Off,
            1 => LightModes::Rainbow,
            2 => LightModes::RainbowSlow,
            3 => LightModes::Mood,
            4 => LightModes::Blue,
            5 => LightModes::BlueMood,
            6 => LightModes::Flame,
            7 => LightModes::White,
            8 => LightModes::Color,
            9 => LightModes::Orange,
            _ => LightModes::None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct LampData {
    mode: LightModes,
    speed: u16,
    brightness: u16,
    color: (u8, u8, u8),
}

impl Default for LampData {
    fn default() -> Self {
        Self {
            mode: LightModes::Off,
            speed: 100,
            brightness: 100,
            color: (125, 125, 0),
        }
    }
}
#[derive(Serialize)]
struct Modes {
    name: String,
    value: u32,
    label: String,
}

impl From<(&str, u32, &str)> for Modes {
    fn from(val: (&str, u32, &str)) -> Self {
        Self {
            name: String::from(val.0),
            value: val.1,
            label: String::from(val.2),
        }
    }
}

// State of an network controled lamp
struct Lamp {
    name: String,
    topic_cmd: String,
    topic_connect: String,
    title: String,
    data: RwLock<LampData>,
    file: String,
    modes: Vec<Modes>,
}

struct LampState {
    lamps: Vec<Lamp>,
    mqtt_client: AsyncClient,
}

impl LampState {
    pub fn get_lamp(&self, name: &str) -> Option<&Lamp> {
        self.lamps.iter().find(|l| name == l.name.to_lowercase())
    }

    pub fn new_arc(lamps: Vec<Lamp>) -> Arc<Mutex<Self>> {
        let mqtt_options = MqttOptions::new("RocketServer", "AlexZero", 1883);
        let (mqtt_client, mut ev) = AsyncClient::new(mqtt_options, 20);
        let qos = QoS::AtLeastOnce;
        let topics: Vec<_> = lamps
            .iter()
            .map(|lamp| lamp.topic_connect.clone())
            .collect();
        let c = mqtt_client.clone();
        let s = Arc::new(Mutex::new(Self { lamps, mqtt_client }));
        let lamp = s.clone();
        rocket::tokio::spawn(async move {
            for t in topics {
                c.subscribe(t, qos).await.unwrap();
            }
            // Wait for incoming events
            loop {
                match ev.poll().await {
                    Ok(notification) => match notification {
                        rumqttc::Event::Incoming(nf) => match nf {
                            rumqttc::Incoming::Publish(p) => {
                                info!("Topic: {}, Payload: {:?}", p.topic, p.payload);
                                let data = {
                                    let lamp = lamp.lock().await;
                                    let lamp =
                                        lamp.lamps.iter().find(|l| l.topic_connect == p.topic);
                                    if let Some(l) = lamp {
                                        Some((l.data.read().await.clone(), l.topic_cmd.clone()))
                                    } else {
                                        None
                                    }
                                };
                                if let Some((data, topic)) = data {
                                    let payload = serde_json::to_string(&data).unwrap();
                                    c.publish(topic, QOS, false, payload).await.unwrap();
                                }
                            }
                            _ => println!("Received = {:?}", nf),
                        },
                        rumqttc::Event::Outgoing(_) => println!("Received = {:?}", notification),
                    },
                    Err(e) => {
                        error!("Mqtt connection erro: {}", e);
                        break;
                    }
                }
            }
        });
        s
    }
}

type SharedLampState = Arc<Mutex<LampState>>;

// Load Lamp definitions
fn load_lamps() -> Arc<Mutex<LampState>> {
    let bar_file = "bar.json";
    let bar_data = if let Ok(data) = File::open(bar_file) {
        // If no file exists use default values
        serde_json::from_reader(data).unwrap_or_default()
    } else {
        LampData::default()
    };
    let bar = Lamp {
        name: String::from("Bar"),
        topic_cmd: String::from("/bar/light"),
        topic_connect: String::from("/bar/connect"),
        title: String::from("Bar"),
        file: String::from(bar_file),
        data: RwLock::new(bar_data),
        modes: vec![
            Modes::from(("off", 0, "Off")),
            Modes::from(("rainbow", 1, "Rainbow")),
            Modes::from(("rainbow_slow", 2, "Slow")),
            Modes::from(("mood", 3, "Mood")),
            Modes::from(("blue", 4, "Blue")),
            Modes::from(("blue_mode", 5, "Mood Blue")),
            Modes::from(("flame", 6, "Flame")),
        ],
    };
    let tv_file = "tv.json";
    let tv_data = if let Ok(data) = File::open(tv_file) {
        // If no file exists use default values
        serde_json::from_reader(data).unwrap_or_default()
    } else {
        LampData::default()
    };

    let tv = Lamp {
        name: String::from("Tv"),
        topic_cmd: String::from("/rgbw1/light"),
        topic_connect: String::from("/rgbw1/connect"),
        title: String::from("Tv"),
        file: String::from(tv_file),
        data: RwLock::new(tv_data),
        modes: vec![
            Modes::from(("off", 0, "Off")),
            Modes::from(("rainbow", 1, "Rainbow")),
            Modes::from(("rainbow_slow", 2, "Slow")),
            Modes::from(("mood", 3, "Mood")),
            Modes::from(("blue", 4, "Blue")),
            Modes::from(("blue_mode", 5, "Mood Blue")),
            //Modes::from(("flame", 6)) // Flame not supported by this lamp
            Modes::from(("white", 7, "White")),
            Modes::from(("color", 8, "Color")),
            Modes::from(("orange", 9, "Orange")),
        ],
    };
    LampState::new_arc(vec![bar, tv])
}

#[get("/")]
fn index() -> &'static str {
    "Not supported"
}

#[derive(Serialize)]
struct TemplateContext<'a> {
    title: &'a str,
    modes: &'a Vec<Modes>,
    activemode: u32,
    brigthness: u16,
    speed: u16,
    color: String,
    data: String,
    url: String,
}

// generate html page for each lamp from template
#[get("/<name>")]
async fn index_lamp(name: &str, data: &State<SharedLampState>) -> Result<Template, Status> {
    let data = data.lock().await;
    if let Some(lamp) = data.get_lamp(name) {
        let context = {
            let data = lamp.data.write().await;
            let cur = data.mode.clone() as u32;
            let activemode = lamp
                .modes
                .iter()
                .find_map(|m| if cur == m.value { Some(m.value) } else { None })
                .unwrap_or(0);
            println!("{}", activemode);
            TemplateContext {
                title: lamp.title.as_str(),
                modes: &lamp.modes,
                activemode,
                brigthness: data.brightness,
                speed: data.speed,
                color: format!(
                    "#{:02x}{:02x}{:02x}",
                    data.color.0, data.color.1, data.color.2
                ),
                data: serde_json::to_string(&data.clone()).unwrap_or_else(|_| String::from("")),
                url: lamp.name.to_lowercase(),
            }
        };
        Ok(Template::render("controls", &context))
    } else {
        warn_!("Led: {} unkown", name);
        Err(Status::NotFound)
    }
}

// convert a u32 to rgb
fn transform_u32_to_color(x: u32) -> (u8, u8, u8) {
    let b1: u8 = ((x >> 16) & 0xff) as u8;
    let b2: u8 = ((x >> 8) & 0xff) as u8;
    let b3: u8 = (x & 0xff) as u8;
    (b1, b2, b3)
}

// update a single value and return the current state as json
#[get("/update/<name>/<key>/<value>")]
async fn update_lamp(
    name: &str,
    key: &str,
    value: String,
    sdata: &State<SharedLampState>,
) -> Result<Json<LampData>, Status> {
    let sdata = sdata.lock().await;
    if let Some(lamp) = sdata.get_lamp(name) {
        let mut data = lamp.data.write().await;
        match key {
            "mode" => {
                if let Ok(m) = LightModes::try_from(value) {
                    data.mode = m;
                }
            }
            "speed" => {
                if let Ok(s) = value.parse::<u16>() {
                    if s > 0 && s <= 512 {
                        data.speed = s;
                    }
                }
            }
            "brigthness" => {
                if let Ok(b) = value.parse::<u16>() {
                    if b <= 255 {
                        data.brightness = b;
                    }
                }
            }
            "color" => {
                let value = value.trim_start_matches('#');
                if let Ok(col) = u32::from_str_radix(value, 16) {
                    data.color = transform_u32_to_color(col);
                }
            }
            _ => {
                warn_!("Unkown Key{}", key);
            }
        }

        let file = lamp.file.clone();
        let d = data.clone();
        let json = serde_json::to_string(&d).expect("Error");
        if let Ok(mut f) = rocket::tokio::fs::File::create(file).await {
            f.write_all(json.as_bytes()).await.expect("write file");
        }
        sdata
            .mqtt_client
            .publish(lamp.topic_cmd.clone(), QOS, false, json.as_bytes())
            .await
            .expect("mqtt send");
        Ok(Json(data.clone()))
    } else {
        warn_!("Unkown Lamp {}", key);
        Err(Status::NotFound)
    }
}

#[launch]
fn rocket() -> _ {
    let lamps = load_lamps();

    rocket::build()
        .mount("/", routes![index, index_lamp, update_lamp])
        .mount("/static", FileServer::from(relative!("static")))
        .manage(lamps)
        .attach(Template::custom(|_engines| {}))
}
