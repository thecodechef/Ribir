use std::time::Duration;

use ribir::prelude::*;

pub fn counter() -> Widget {
  widget! {
    states { cnt: Stateful::new(0) }
    Column {
      h_align: HAlign::Center,
      align_items: Align::Center,
      FilledButton { on_tap: move |_| *cnt += 1, Label::new("Add") }
      H1 { text: cnt.to_string() }
      FilledButton { on_tap: move |_| *cnt += -1, Label::new("Sub") }

      widget::from(image_loading())
    }
  }
}

fn image_loading() -> Widget {
  widget! {
    init {
      let img = include_bytes!("./image_loading.png").to_vec();
      let img = PixelImage::from_png(&img);
      let img = ShareResource::new(img);
      let loading = include_bytes!("./loading.png").to_vec();
      let loading = PixelImage::from_png(&loading);
      let loading = ShareResource::new(loading);
    }

    Stack {
      LayoutBox {
        id: loading_brand,
        widget::from(img)
      }
      Container {
        id: container,
        size: Size::zero(),
        Column {
          v_align: VAlign::Center,
          h_align: HAlign::Center,
          align_items: Align::Center,
          DynWidget {
            id: loading_icon,
            transform: Transform::default(),
            on_mounted: move |_| {
              loading_animate.run();
            },
            dyns: loading
          }
          Text {
            text: "Loading...",
          }
        }
      }
    }

    finally {
      let_watch!(loading_brand.layout_size())
        .subscribe(move |size| container.size = size);
    }

    Animate {
      id: loading_animate,
      transition: Transition {
        delay: None,
        duration: Duration::from_millis(1000),
        easing: easing::LINEAR,
        repeat: Some(f32::MAX),
      },
      prop: prop!(loading_icon.transform, move |_, _, rate| {
        let mut transform = Transform::default();
        let w = loading_icon.layout_width();
        let h = loading_icon.layout_height();

        transform
          .pre_translate(Vector::new(w/2.0, h/2.0))
          .pre_rotate(Angle::two_pi() * rate)
          .pre_translate(Vector::new(-w/2.0, -h/2.0))
      }),
      from: Transform::default(),
    }
  }
}

fn text_loading() -> Widget {
  widget! {
    states {
      text: Stateful::new("loading"),
    }
    Text {
      id: label,
      text: text.to_owned(),
      on_mounted: move |_| loading.run(),
    }
    Animate {
      id: loading,
      transition: Transition {
        delay: None,
        duration: Duration::from_millis(1000),
        easing: easing::LINEAR,
        repeat: Some(f32::MAX),
      },
      prop: prop!(label.text, move |_, _, rate| {
        let mut text = String::from("loading");
        for _ in 0..=(rate * 3.) as usize {
          text.push('.');
        }
        text.into()
      }),
      from: "loading".into(),
    }
  }
}
