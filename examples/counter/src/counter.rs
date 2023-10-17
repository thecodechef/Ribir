use ribir::prelude::*;

pub fn counter() -> impl WidgetBuilder {
  fn_widget! {
    let count = State::value(0);

    @Row {
      @FilledButton {
        on_tap: move |_| *$count.write() += 1,
        @ { Label::new("Increment") }
      }
      @H1 { text: pipe!($count.to_string()) }
    }
  }
}
